//! Reprise Blue: its colour schemes ([`Variant`], each with a named [`scheme`] of colours and
//! sizes), the painters that draw every part from them in code at the output scale, and the
//! style they make.

mod bar;
mod capbutton;
mod controls;
mod frame;
pub mod overview;
pub mod scheme;
mod start;
mod taskbutton;
mod trayarrow;

use std::sync::{Arc, OnceLock};

use crate::metrics::{ControlMetrics, FileDlgMetrics, StartMenuMetrics};
use crate::raster::Insets;
use crate::raster::Rect;
use crate::style::{Part, Shadow, SoftShadow, State, Style, TextSpec, TrackMarks};
use crate::taskbar::{ClockBox, LabelledButton, StartMenuDock, TaskbarShape, TrayArrow};
use crate::theme::{Color, Metrics};

/// Env var carrying Blue's colour scheme to clients, spelled as [`Variant::as_str`], beside
/// [`crate::interface::ENV`].
pub const ENV: &str = "REPRISE_BLUE_VARIANT";

/// One of Blue's colour schemes. Every variant draws the same parts at the same sizes; only
/// pixels and system colours differ.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Variant {
    #[default]
    Blue,
    Olive,
    Silver,
    Slate,
    Sapphire,
    Obsidian,
    Ember,
}

impl Variant {
    /// Every variant in declaration order, the order a scheme list shows: `v as usize` indexes it.
    pub const ALL: [Variant; 7] = [
        Variant::Blue,
        Variant::Olive,
        Variant::Silver,
        Variant::Slate,
        Variant::Sapphire,
        Variant::Obsidian,
        Variant::Ember,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Variant::Blue => "Blue",
            Variant::Olive => "Olive Green",
            Variant::Silver => "Silver",
            Variant::Slate => "Slate",
            Variant::Sapphire => "Sapphire",
            Variant::Obsidian => "Obsidian",
            Variant::Ember => "Ember",
        }
    }

    pub fn from_name(name: &str) -> Option<Variant> {
        Variant::ALL.into_iter().find(|v| v.name() == name)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Variant::Blue => "blue",
            Variant::Olive => "olive",
            Variant::Silver => "silver",
            Variant::Slate => "slate",
            Variant::Sapphire => "sapphire",
            Variant::Obsidian => "obsidian",
            Variant::Ember => "ember",
        }
    }

    pub fn parse(s: &str) -> Option<Variant> {
        Variant::ALL.into_iter().find(|v| v.as_str() == s)
    }

    /// The variant an [`ENV`] value names; unset or unknown is navy.
    pub fn from_env_value(v: Option<&str>) -> Variant {
        v.and_then(Variant::parse).unwrap_or_default()
    }

    /// The style's name, which tells variants apart ([`crate::look::Look`] compares styles by
    /// name).
    fn style_name(self) -> &'static str {
        match self {
            Variant::Blue => "Blue",
            Variant::Olive => "Blue Olive Green",
            Variant::Silver => "Blue Silver",
            Variant::Slate => "Blue Slate",
            Variant::Sapphire => "Blue Sapphire",
            Variant::Obsidian => "Blue Obsidian",
            Variant::Ember => "Blue Ember",
        }
    }
}

/// Reprise Blue's style in its navy scheme, built on first use and shared.
pub fn style() -> Arc<Style> {
    style_for(Variant::Blue)
}

/// Blue's style in `v`'s scheme, built on first use and shared. At another scale it is built
/// again ([`Style::at_scale`]).
pub fn style_for(v: Variant) -> Arc<Style> {
    static STYLES: [OnceLock<Arc<Style>>; Variant::ALL.len()] =
        [const { OnceLock::new() }; Variant::ALL.len()];
    STYLES[v as usize]
        .get_or_init(|| Arc::new(build(v, 1).with_rescale(move |n| build(v, n))))
        .clone()
}

/// `v`'s style at `scale` device pixels per design pixel. Parts painted from geometry are
/// painted at `scale` and marked so
/// ([`PartImage::at_scale`](crate::style::PartImage::at_scale)); other parts are design-size
/// art the draw magnifies.
fn build(v: Variant, scale: i32) -> Style {
    debug_assert!(scale >= 1, "a style for scale {scale}");
    let inks = v.scheme().inks;
    let style = Style::new(
        v.style_name(),
        v.scheme().system.colors(),
        metrics(v),
        caption_text(v),
        start_text(v),
        v.scheme().inks.bar_text,
        taskbar_shape(),
    )
    .with_controls(controls(v))
    .with_flat_menus()
    .with_track_marks(TrackMarks {
        ink: inks.track_tick,
        ..TrackMarks::LOW
    })
    .with_text_color(Part::PushButton, State::Disabled, inks.greyed_label)
    .with_text_color(Part::CheckBox, State::Disabled, inks.greyed_label)
    .with_text_color(Part::Radio, State::Disabled, inks.greyed_label)
    .with_text_color(Part::GroupBox, State::Normal, inks.group_label)
    .with_text_color(Part::CalendarBand, State::Normal, inks.calendar_letters)
    .with_text_color(Part::CalendarBand, State::Disabled, inks.calendar_letters)
    .with_text_color(Part::TaskButton, State::Normal, inks.task_normal)
    .with_text_color(Part::TaskButton, State::Hot, inks.task_hot)
    .with_text_color(Part::TaskButton, State::Checked, inks.task_checked)
    .with_text_color(Part::TaskButton, State::CheckedHot, inks.task_checked_hot)
    .with_text_color(Part::TaskButton, State::Pressed, inks.task_pressed)
    .with_painted(Part::Caption, move || frame::caption(v.scheme(), scale))
    .with_painted(Part::MaxCaption, move || {
        frame::max_caption(v.scheme(), scale)
    })
    .with_painted(Part::FrameLeft, move || {
        frame::frame_left(v.scheme(), scale)
    })
    .with_painted(Part::FrameRight, move || {
        frame::frame_right(v.scheme(), scale)
    })
    .with_painted(Part::FrameBottom, move || {
        frame::frame_bottom(v.scheme(), scale)
    });
    let style = [
        (Part::MinButton, capbutton::Glyph::Minimize),
        (Part::MaxButton, capbutton::Glyph::Maximize),
        (Part::RestoreButton, capbutton::Glyph::Restore),
        (Part::CloseButton, capbutton::Glyph::Close),
    ]
    .into_iter()
    .fold(style, |st, (part, glyph)| {
        st.with_painted(part, move || {
            capbutton::caption_button(v.scheme(), glyph, scale)
        })
    })
    .with_painted(Part::Taskbar, move || bar::taskbar(v.scheme(), scale))
    .with_painted(Part::StartButton, move || {
        start::start_button(v.scheme(), scale)
    })
    .with_painted(Part::TaskButton, move || taskbutton::task_button(v, scale))
    .with_painted(Part::TrayArea, move || bar::tray_area(v.scheme(), scale));
    let style = controls::toolbar::inks(v)
        .into_iter()
        .fold(style, |st, (part, state, ink)| {
            st.with_text_color(part, state, ink)
        });
    let style = controls::eager(v)
        .into_iter()
        .fold(style, |st, (part, img)| st.with_part(part, img));
    let style = controls::GROUPS
        .into_iter()
        .fold(style, |st, (parts, paint)| {
            debug_assert!(
                !parts.iter().any(|&p| st.has_parts(&[p])),
                "blue control parts {parts:?} collide with a part already in the style"
            );
            st.with_painted_group(parts, move || paint(v, scale))
        });
    let style = controls::inks(v)
        .into_iter()
        .fold(style, |st, (part, state, ink)| {
            st.with_text_color(part, state, ink)
        });
    let tabs = caption_tabs(&style.caption_text);
    let arrow = trayarrow::art(&trayarrow::recipe(v), scale);
    style.with_caption_tabs(tabs).with_tray_arrow(arrow)
}

/// Blue's caption tabs over a band whose titles are inked as `title`: rounded 3 px washes,
/// white under a light title and black under a dark one (silver's), alphas out of FF. Light:
/// selected 40 inside a line of 80, pointer 24, held window 30. Dark: 18, line 60, 10, 14.
/// Other tabs use the band's inactive ink. The close box is 14 px, a wash of 48 (dark 20) under
/// the pointer and 30 of black held.
fn caption_tabs(title: &crate::style::TextSpec) -> crate::style::CaptionTabLook {
    let white = Color::rgb(0xFF, 0xFF, 0xFF);
    let black = Color::rgb(0, 0, 0);
    let c = title.color;
    let light = u32::from(c.r) + u32::from(c.g) + u32::from(c.b) > 3 * 0x80;
    let (wash, a) = if light {
        (white, [0x40, 0x24, 0x30, 0x48])
    } else {
        (black, [0x18, 0x10, 0x14, 0x20])
    };
    crate::style::CaptionTabLook {
        radius: 3,
        selected: (wash, a[0]),
        outline: (white, if light { 0x80 } else { 0x60 }),
        hot: (wash, a[1]),
        rest: (wash, 0),
        ghost: (wash, a[2]),
        close: 14,
        close_hot: (wash, a[3]),
        close_held: (black, 0x30),
        other_ink: (title.inactive, title.inactive),
    }
}

/// Blue's pixel metrics in `v`: taller caption and taskbar than Classic's, square caption
/// buttons with a gap, and the Start pill as wide as `v`'s metrics make it
/// ([`scheme::Metrics::start_width`]).
fn metrics(v: Variant) -> Metrics {
    Metrics {
        frame_width: 4,
        dialog_frame_width: 3,
        caption_height: 26,
        caption_button_width: 21,
        caption_button_height: 21,
        caption_button_right_margin: 2,
        caption_button_gap: 2,
        taskbar_height: 30,
        start_button_width: v.scheme().metrics.start_width(),
        task_button_max_width: 160,
        resize_border: 4,
        ..Metrics::default()
    }
}

/// Blue's control sizes: 13 px radio and check box with labels 16 px from the box's left edge
/// (a wrapped check box label a pixel higher), the radio circle in the check box's column, 17
/// px scrollbar with a 10 px minimum thumb, 15 px combo drop button 3 px clear of the
/// highlight, 15 px buddy spinner buttons inside the field, 22 px vertical trackbar thumb, 20
/// px list header, 23 px status bar, `v`'s own task pane, explorer bar, rebar, toolbar strip,
/// menu bars, address band and places bar, a file dialog with rows a pixel higher, and its own
/// Start menu borders.
fn controls(v: Variant) -> ControlMetrics {
    ControlMetrics {
        radio: 13,
        radio_x: -1,
        radio_label: 15,
        check: 13,
        check_label: 16,
        check_wrap: 16,
        check_wrap_dy: -1,
        scroll: 17,
        thumb_min: 10,
        combo_drop: 15,
        combo_gap: 3,
        combo_row: 13,
        list_row: 17,
        buddy_spin: 15,
        buddy_spin_inside: true,
        vtrack_thumb: 22,
        list_header: 20,
        header_arrow_end: 2,
        header_arrow_over: false,
        status_bar: 23,
        flyout_border: 2,
        task: controls::task_metrics(v),
        bar: controls::bar_metrics(),
        rebar: controls::rebar_metrics(v),
        toolbar: controls::toolbar_metrics(),
        menubar: controls::menubar_metrics(),
        address: controls::address_metrics(),
        places: controls::places_metrics(),
        file_dlg: FileDlgMetrics { dy: -1 },
        start_menu: start_menu_metrics(),
        text: crate::theme::TextUnits::ONE,
    }
}

/// Blue's Start menu: a 1 px line, then a row of `menu` above the rows and nothing on the other
/// sides, the band right inside the line; a submenu's rows right inside its line. A separator
/// stops 2 px short of each end. A lit row's submenu arrow stays black (`menu_text`) in every
/// variant.
fn start_menu_metrics() -> StartMenuMetrics {
    StartMenuMetrics {
        root: Insets::new(1, 2, 1, 1),
        sub: Insets::new(1, 1, 1, 1),
        separator_x: 2,
        lit_arrow_menu_text: true,
    }
}

/// The window title's bold face in `v`: the scheme's title inks with a hard shadow 1 px down
/// and right, 13 px (11 px in slate and ember).
fn caption_text(v: Variant) -> TextSpec {
    let s = v.scheme();
    TextSpec {
        family: vec!["Trebuchet MS".into(), "DejaVu Sans".into()],
        size_px: s.metrics.title_px.unsigned_abs(),
        family_sizes: Vec::new(),
        bold: true,
        italic: false,
        antialias: true,
        full_hinting: true,
        color: s.inks.title,
        inactive: s.inks.title_inactive,
        shadow: Some(Shadow {
            color: s.inks.title_shadow,
            dx: 1,
            dy: 1,
            soft: None,
        }),
        ink_gamma: 1.0,
    }
}

/// The Start pill's italic face with a soft dark shadow (`start_shadow`, black in ember):
/// Franklin Gothic Medium Italic at 19 px when installed, else Trebuchet MS Bold Italic at 16
/// px, either inking "start" 37 px wide. `bold` picks the heaviest face and the ink is set
/// heavier still (coverage to the power 1/2). The shadow spreads each inked pixel over a 4x4
/// block from 1 px up-left to 2 px down-right, at 3/2 strength. The pill never goes inactive,
/// so `inactive` is the same label ink.
fn start_text(v: Variant) -> TextSpec {
    let inks = v.scheme().inks;
    TextSpec {
        family: vec![
            "Franklin Gothic Medium".into(),
            "Trebuchet MS".into(),
            "DejaVu Sans".into(),
        ],
        size_px: 16,
        family_sizes: vec![("Franklin Gothic Medium".into(), 19)],
        bold: true,
        italic: true,
        antialias: true,
        full_hinting: true,
        color: inks.start_label,
        inactive: inks.start_label,
        shadow: Some(Shadow {
            color: inks.start_shadow,
            dx: 1,
            dy: 1,
            soft: Some(SoftShadow {
                size: 4,
                gain: (3, 2),
            }),
        }),
        ink_gamma: 0.5,
    }
}

/// Blue's taskbar shape: Start fills the bar's height with an 18 px flag, task buttons sit 3 px
/// down with 2 px below, and the notification well runs the full height to the screen edge,
/// from 11 px left of the first tray icon. Tray icons are 2 px apart, 8 px down a 30 px bar.
/// The tray arrow is a 19 px round button 1 px before the first icon, its middle column on the
/// well's left edge; down a side bar it is centred across the well less its 3 px inner edge,
/// its middle row 1 px under the well's top, 2 px over the first icon row. The clock is 48 px
/// wide, room for any 12-hour time, and ends 12 px in. The Start menu reaches 4 px into the
/// bar.
fn taskbar_shape() -> TaskbarShape {
    TaskbarShape {
        start: Rect::new(0, 0, 0, 30),
        button_top: 3,
        button_bottom: 2,
        start_gap: 7,
        well_full_height: true,
        well_lead: 11,
        tray_gap: 2,
        tray_low: true,
        side_tray_inset: 0,
        tray_arrow: Some(TrayArrow {
            size: (19, 19),
            gap: 1,
            well_lead: -9,
            side_size: (Some(19), 19),
            side_gap: 2,
            side_lead: -8,
            side_inset: 3,
        }),
        clock: ClockBox {
            pad: 2,
            min_width: 48,
            right: 12,
        },
        start_icon: 18,
        task_icon: crate::icon::ICON_SIZE,
        icon_button: None,
        show_desktop_width: None,
        orb: false,
        orb_rise: None,
        two_line_clock: None,
        start_menu: StartMenuDock::Into(4),
        labelled: LabelledButton::CLASSIC,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::Part;

    #[test]
    fn blue_has_every_frame_and_taskbar_part() {
        let s = style();
        for p in [
            Part::Caption,
            Part::MaxCaption,
            Part::FrameLeft,
            Part::FrameRight,
            Part::FrameBottom,
            Part::MinButton,
            Part::MaxButton,
            Part::RestoreButton,
            Part::CloseButton,
            Part::Taskbar,
            Part::StartButton,
            Part::TaskButton,
            Part::TrayArea,
        ] {
            assert!(s.part(p).is_some(), "Blue has {p:?}");
        }
    }

    /// Parts drawn from shapes pin their own pixels at scale 1 and 2, so the doubling checks
    /// leave them out.
    const REDRAWN: &[Part] = &[
        Part::StatusBar,
        Part::StatusPane,
        Part::StartButton,
        Part::ProgressBar,
        Part::ProgressChunk,
        Part::GroupBox,
        Part::SizeBox,
        Part::Caption,
        Part::MaxCaption,
        Part::FrameLeft,
        Part::FrameRight,
        Part::FrameBottom,
        Part::Taskbar,
        Part::TrayArea,
        Part::TaskButton,
        Part::TaskPane,
        Part::TaskGroupHeader,
        Part::TaskGroupBody,
        Part::TaskCollapse,
        Part::TaskExpand,
        Part::TaskCollapseMark,
        Part::TaskExpandMark,
    ];

    /// Every part in every variant, each frame, drawn at s = 2 is its s = 1 draw doubled, at
    /// the sizes hosts pass: square buttons and arrows of 13, 16, 17 and 21, odd bands, short
    /// or narrow rects.
    #[test]
    fn every_blue_part_doubles_at_scale_2_in_each_variant() {
        use crate::look::styled::fixture::part_doubles_at_scale_2;
        let sizes = [
            (13, 13),
            (16, 16),
            (17, 17),
            (21, 21),
            (16, 41),
            (41, 16),
            (22, 23),
            (75, 23),
            (120, 30),
            (7, 30),
            (40, 5),
        ];
        for v in Variant::ALL {
            let st = style_for(v);
            for part in st.part_keys().filter(|p| !REDRAWN.contains(p)) {
                for (w, h) in sizes {
                    if let Err(e) = part_doubles_at_scale_2(&st, part, w, h) {
                        panic!("{v:?}: {e} does not double at s = 2");
                    }
                }
            }
        }
    }

    /// A Blue style drawn at 2 is rebuilt for scale 2; its remaining design-size art draws as
    /// the scale-1 part doubled.
    #[test]
    fn blue_style_at_scale_2_matches_doubled_parts() {
        use crate::raster::{Canvas, Rect};
        let sizes = [(21, 21), (75, 23), (120, 30), (7, 30), (300, 34)];
        let sorted = |st: &Style| {
            let mut keys: Vec<Part> = st.part_keys().collect();
            keys.sort_by_key(|p| format!("{p:?}"));
            keys
        };
        for v in Variant::ALL {
            let one = style_for(v);
            let two = one.at_scale(2);
            assert!(!Arc::ptr_eq(&one, &two), "{v:?}: built again for 2");
            assert!(Arc::ptr_eq(&two, &one.at_scale(2)), "{v:?}: kept");
            assert_eq!(sorted(&one), sorted(&two), "{v:?}: the same parts");
            for part in sorted(&one).into_iter().filter(|p| !REDRAWN.contains(p)) {
                // A part painted at 2 is drawn anew; its own pins hold it.
                if two.part(part).unwrap().scale > 1 {
                    continue;
                }
                let img = one.part(part).unwrap();
                for (i, &state) in img.states.iter().enumerate() {
                    let nth = img.states[..i].iter().filter(|&&s| s == state).count();
                    let check = img.checks.get(i).copied();
                    for (w, h) in sizes {
                        let r = Rect::new(0, 0, 2 * w, 2 * h);
                        let paint = |st: &Style| {
                            let mut c = Canvas::new(r.width as u32, r.height as u32);
                            match check {
                                Some(k) => st.draw_check(&mut c, r, part, k, state, 2),
                                None => st.draw_nth(&mut c, r, part, state, nth, 2),
                            }
                            c.pixels
                        };
                        assert!(
                            paint(&one) == paint(&two),
                            "{v:?} {part:?} {state:?} #{nth} at {w}x{h}"
                        );
                    }
                }
            }
        }
    }

    /// Each part cuts its atlas into one equal frame per listed state; an unlisted state (push
    /// button Default and Focused) draws the first frame.
    #[test]
    fn blue_parts_draw_one_equal_frame_per_listed_state() {
        use crate::style::{Layout, State};
        let s = style();
        for part in [
            Part::Caption,
            Part::MaxCaption,
            Part::FrameLeft,
            Part::FrameRight,
            Part::FrameBottom,
            Part::MinButton,
            Part::MaxButton,
            Part::RestoreButton,
            Part::CloseButton,
            Part::Taskbar,
            Part::StartButton,
            Part::TaskButton,
            Part::TrayArea,
        ] {
            let p = s.part(part).unwrap();
            assert!(p.checks.is_empty(), "{part:?} has no check frames");
            let n = p.states.len() as i32;
            let (w, h) = (p.atlas.width as i32, p.atlas.height as i32);
            let nth = |i: i32| match p.layout {
                Layout::Vertical => Rect::new(0, i * (h / n), w, h / n),
                Layout::Horizontal => Rect::new(i * (w / n), 0, w / n, h),
            };
            for (i, &state) in p.states.iter().enumerate() {
                assert_eq!(
                    p.frame(state),
                    nth(i as i32),
                    "{part:?} {state:?} is frame {i}"
                );
            }
            for state in [State::Default, State::Focused] {
                assert_eq!(p.frame(state), nth(0), "{part:?} {state:?} is frame 0");
            }
        }
    }

    #[test]
    fn every_variant_round_trips_through_its_name_and_its_value() {
        for v in Variant::ALL {
            assert_eq!(Variant::from_name(v.name()), Some(v));
            assert_eq!(Variant::parse(v.as_str()), Some(v));
        }
        assert_eq!(Variant::from_name("Olive Green"), Some(Variant::Olive));
        assert_eq!(Variant::parse("olive"), Some(Variant::Olive));
        assert_eq!(Variant::from_name("Sapphire"), Some(Variant::Sapphire));
        assert_eq!(Variant::parse("ember"), Some(Variant::Ember));
        assert_eq!(Variant::parse("green"), None);
        assert_eq!(Variant::default(), Variant::Blue);
    }

    #[test]
    fn an_unset_or_unknown_env_value_is_navy() {
        assert_eq!(Variant::from_env_value(Some("silver")), Variant::Silver);
        assert_eq!(Variant::from_env_value(Some("slate")), Variant::Slate);
        assert_eq!(Variant::from_env_value(Some("Silver")), Variant::Blue);
        assert_eq!(Variant::from_env_value(Some("green")), Variant::Blue);
        assert_eq!(Variant::from_env_value(None), Variant::Blue);
    }

    /// Each variant is its own look with every part navy has, so no control falls back to Classic.
    #[test]
    fn every_variant_has_every_navy_part_under_its_own_name() {
        let navy = style();
        let names: std::collections::HashSet<_> = Variant::ALL.map(|v| style_for(v).name).into();
        assert_eq!(names.len(), Variant::ALL.len(), "one distinct look each");
        for v in Variant::ALL {
            let s = style_for(v);
            assert!(Arc::ptr_eq(&s, &style_for(v)), "{v:?} is built once");
            for part in navy.part_keys() {
                assert!(s.part(part).is_some(), "{v:?} has {part:?}");
            }
        }
        assert!(Arc::ptr_eq(&navy, &style_for(Variant::Blue)));
    }

    #[test]
    fn blue_frame_is_four_px_with_a_thirty_px_top() {
        let m = &style().metrics;
        assert_eq!((m.frame_width, m.frame_width + m.caption_height), (4, 30));
        assert_eq!(
            (
                m.caption_button_width,
                m.caption_button_height,
                m.caption_button_gap
            ),
            (21, 21, 2)
        );
        assert_eq!(m.taskbar_height, 30);
    }

    /// The Start pill is as wide as each variant's art: 97 px in navy, olive green and silver,
    /// 99 px in slate, sapphire, obsidian and ember.
    #[test]
    fn the_start_pill_is_as_wide_as_its_art() {
        for (v, w) in [
            (Variant::Blue, 97),
            (Variant::Olive, 97),
            (Variant::Silver, 97),
            (Variant::Slate, 99),
            (Variant::Sapphire, 99),
            (Variant::Obsidian, 99),
            (Variant::Ember, 99),
        ] {
            let st = style_for(v);
            assert_eq!(st.metrics.start_button_width, w, "{v:?}");
            let img = st.part(Part::StartButton).unwrap();
            assert_eq!(img.frame(State::Normal).width, w, "{v:?} atlas");
        }
    }

    /// A window title is 13 px (11 px in slate and ember); a group box label inks each
    /// variant's own colour.
    #[test]
    fn the_start_label_asks_for_franklin_gothic_medium_then_trebuchet_ms() {
        let spec = start_text(Variant::Blue);
        assert_eq!(
            spec.family[..2],
            ["Franklin Gothic Medium", "Trebuchet MS"],
            "Franklin Gothic Medium first, Trebuchet MS as its fallback"
        );
        assert!(spec.bold && spec.italic, "the label asks for bold italic");
    }

    /// Width and height of the box "start" inks in `spec`'s face at its size, or `None` when
    /// that face is not `family`.
    fn start_ink(spec: &TextSpec, family: &str) -> Option<(i32, i32)> {
        let t = crate::text::TextRenderer::load(&spec.font());
        if !t.has_face(family) {
            return None;
        }
        let mut c = crate::raster::Canvas::new(100, 30);
        t.draw(
            &mut c,
            Rect::new(0, 0, 100, 30),
            "start",
            spec.bold,
            spec.color,
            crate::text::Align::Left,
            spec.size_in(&t),
        );
        let ink: Vec<(i32, i32)> = (0..30)
            .flat_map(|y| (0..100).map(move |x| (x, y)))
            .filter(|&(x, y)| c.get(x, y) & 0xFF_FFFF != 0)
            .collect();
        let span = |v: Vec<i32>| v.iter().max().unwrap() - v.iter().min().unwrap() + 1;
        Some((
            span(ink.iter().map(|p| p.0).collect()),
            span(ink.iter().map(|p| p.1).collect()),
        ))
    }

    #[test]
    fn the_start_label_inks_37_by_12_px_in_franklin_gothic_medium() {
        let spec = start_text(Variant::Blue);
        let Some(ink) = start_ink(&spec, "Franklin Gothic Medium") else {
            return;
        };
        assert_eq!(ink, (37, 12), "37 px wide, 12 rows tall");
    }

    #[test]
    fn the_start_label_inks_37_px_wide_in_trebuchet_ms_at_16_px() {
        let mut spec = start_text(Variant::Blue);
        spec.family.retain(|f| f != "Franklin Gothic Medium");
        let Some((w, _)) = start_ink(&spec, "Trebuchet MS") else {
            return;
        };
        assert_eq!(spec.size_px, 16);
        assert_eq!(w, 37, "37 px wide");
    }

    #[test]
    fn a_caption_titles_stems_land_on_whole_pixels() {
        let spec = caption_text(Variant::Blue);
        let t = crate::text::TextRenderer::load(&spec.font());
        if !t.has_face("Trebuchet MS") {
            return;
        }
        let mut c = crate::raster::Canvas::new(100, 30);
        c.fill_rect(Rect::new(0, 0, 100, 30), Color::rgb(0x00, 0x55, 0xE5));
        t.draw(
            &mut c,
            Rect::new(0, 0, 100, 30),
            "My Computer",
            spec.bold,
            spec.color,
            crate::text::Align::Left,
            spec.size_px,
        );
        let full = (0..30)
            .flat_map(|y| (0..100).map(move |x| (x, y)))
            .filter(|&(x, y)| c.get(x, y) == spec.color.argb())
            .count();
        assert!(
            full >= 150,
            "at least 150 fully inked title pixels, got {full}"
        );
    }

    #[test]
    fn a_caption_title_is_81_px_wide_in_its_own_face() {
        let spec = caption_text(Variant::Blue);
        let t = crate::text::TextRenderer::load(&spec.font());
        if !t.has_face("Trebuchet MS") {
            return;
        }
        assert_eq!(t.measure("My Computer", spec.bold, spec.size_px), 81);
    }
}
