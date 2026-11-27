//! Dark twins: a Blue scheme drawn on charcoal, built from its light scheme by two colour rules
//! and a lightness range per part. Chrome (captions, frames, caption buttons, the taskbar, the
//! Start button, the tray) keeps the light scheme's colours.
//! - Client inks ([`ink`]): OKLab lightness through [`INK`], so white goes charcoal and black
//!   light grey, with chroma scaled by [`INK_CHROMA`] and nudged [`WARM`]. Bevel shades go the
//!   same way, so a shadow line (also an inactive menu bar's ink) is light on charcoal. Selection
//!   and desktop colours are kept.
//! - Control faces ([`face`]): a light body (lightness from [`BODY_FROM`] up) lands on its
//!   part's [`Range`] in order, so gloss keeps its direction; darker outlines turn light through
//!   [`OUTLINE`] and marks through [`INK`]. Saturated colours (the hover rim, the focus and
//!   default rings, check marks, the progress fill) keep their lightness. Halos are kept.
//! - Task panes ([`deep`]): every colour takes the light pane's hue at one low chroma, its
//!   lightness through [`DEEP`], under light titles and links; fold discs stand lighter.
//! - The Start menu's program column follows the window and menu inks; its bands and its
//!   coloured places column are kept. The welcome screen's blues go deeper ([`relit`]).

use std::sync::OnceLock;

use super::*;
use crate::oklab::{color_of, curve, lab_of, smoothstep};

/// `v`'s dark twin, built on first use; a variant without one yet keeps its light scheme.
pub(super) fn twin(v: Variant) -> &'static Scheme {
    static TWINS: [OnceLock<Scheme>; Variant::ALL.len()] =
        [const { OnceLock::new() }; Variant::ALL.len()];
    match v {
        Variant::Blue => TWINS[v as usize].get_or_init(|| darken(&NAVY)),
        _ => scheme(v),
    }
}

/// Light to dark lightness for client inks, linear between knots: text (0) goes light, the
/// dialog face (0.93) and window (1) a warm charcoal, grey text (0.73) a mid grey.
const INK: [(f32, f32); 7] = [
    (0.00, 0.90),
    (0.30, 0.80),
    (0.50, 0.72),
    (0.73, 0.62),
    (0.82, 0.32),
    (0.93, 0.27),
    (1.00, 0.22),
];

/// How much of a light ink's OKLab chroma its dark ink keeps.
const INK_CHROMA: f32 = 0.25;

/// OKLab (a, b) added to every dark ink, so the greys are warm.
const WARM: [f32; 2] = [0.003, 0.006];

/// Lightness from which a face colour is part of the body; below it, an outline or glyph.
const BODY_FROM: f32 = 0.78;

/// Outline lightness, linear between knots up to 0.70, then on to the body's range at
/// [`BODY_FROM`]: dark rims turn light, so a control's outline reads on charcoal.
const OUTLINE: [(f32, f32); 4] = [(0.00, 0.93), (0.35, 0.64), (0.55, 0.58), (0.70, 0.52)];

/// How much OKLab chroma a mapped face colour keeps.
const FACE_CHROMA: f32 = 0.8;

/// Chroma, then lightness, over which a face colour moves from mapped to kept.
const KEEP_CHROMA: (f32, f32) = (0.085, 0.115);
const KEEP_ABOVE: (f32, f32) = (0.42, 0.52);

/// The lightness a part's body runs over in the twin, from its darkest to its lightest colour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Range(pub f32, pub f32);

/// Buttons, arrows, thumbs and headers: a glossy face lighter than the dialog.
const RAISED: Range = Range(0.30, 0.42);
/// Check box and radio wells, the progress trough and a greyed field.
const WELL: Range = Range(0.21, 0.31);
/// Scrollbar and trackbar tracks, darker than their thumbs.
const TRACK: Range = Range(0.19, 0.24);
/// Tabs and their pages (an open tab is its page's colour), status bars, rebars, the explorer
/// bar's header and the list header's filler, near the dialog face.
const PANE: Range = Range(0.25, 0.30);
/// A group box's line, lighter than the dialog it divides.
const LINE: Range = Range(0.42, 0.46);
/// The file dialog's places bar, sunk below the dialog like a list.
const SUNK: Range = Range(0.20, 0.24);

/// Task pane lightness, light to dark, linear between knots: the pane sits under the dialog,
/// its groups' bodies and title bands stand above it.
const DEEP: [(f32, f32); 4] = [(0.55, 0.23), (0.70, 0.26), (0.86, 0.30), (1.00, 0.34)];
/// The OKLab chroma of every deep colour, in the hue of the light pane.
const DEEP_CHROMA: f32 = 0.045;
/// A fold button's disc: its rim and its face, lighter than the title band.
const DISC_RIM: f32 = 0.58;
const DISC_FACE: f32 = 0.42;
/// Task pane titles and links, and the fold chevrons, at rest and lit: (lightness, chroma).
const TASK_INK: (f32, f32) = (0.86, 0.06);
const TASK_INK_HOT: (f32, f32) = (0.98, 0.015);
/// The welcome screen's bands, field, glow and lit tile band: (lightness, share of chroma kept).
const WELCOME_BAND: (f32, f32) = (0.22, 0.6);
const WELCOME_FIELD: (f32, f32) = (0.31, 0.6);
const WELCOME_GLOW: (f32, f32) = (0.37, 0.6);
const WELCOME_TILE_HOT: (f32, f32) = (0.44, 0.7);
/// A tooltip's lightness: above the charcoal windows it pops over.
const INFO_BK: f32 = 0.31;

/// `c` as a dark client ink.
pub(super) fn ink(c: Color) -> Color {
    let [l, a, b] = lab_of(c);
    color_of([
        curve(&INK, l),
        a * INK_CHROMA + WARM[0],
        b * INK_CHROMA + WARM[1],
    ])
}

/// `c` at lightness `l`, keeping share `k` of its chroma.
fn relit(c: Color, (l, k): (f32, f32)) -> Color {
    let [_, a, b] = lab_of(c);
    color_of([l, a * k, b * k])
}

/// The unit OKLab hue of `c`.
fn hue(c: Color) -> [f32; 2] {
    let [_, a, b] = lab_of(c);
    let n = a.hypot(b).max(f32::EPSILON);
    [a / n, b / n]
}

/// A colour of hue `h` at lightness `l` and chroma `c`.
fn tinted_at(h: [f32; 2], (l, c): (f32, f32)) -> Color {
    color_of([l, h[0] * c, h[1] * c])
}

/// A task pane colour in the twin: lightness through [`DEEP`], in hue `h`.
fn deep(c: Color, h: [f32; 2]) -> Color {
    tinted_at(h, (curve(&DEEP, lab_of(c)[0]), DEEP_CHROMA))
}

fn deep_face(f: &Face, h: [f32; 2]) -> Face {
    let grad = |g: Grad| leak(g.iter().map(|&(t, c)| (t, deep(c, h))).collect());
    Face {
        rim: deep(f.rim, h),
        inner: deep(f.inner, h),
        halo: f.halo,
        down: grad(f.down),
        across: grad(f.across),
        mark: f.mark,
    }
}

/// A fold button in the twin: a disc lighter than its band, its chevron `mark`.
fn disc(f: &Face, h: [f32; 2], mark: (f32, f32)) -> Face {
    Face {
        rim: tinted_at(h, (DISC_RIM, DEEP_CHROMA)),
        inner: tinted_at(h, (DISC_FACE, DEEP_CHROMA)),
        mark: Some(tinted_at(h, mark)),
        ..deep_face(f, h)
    }
}

/// How far `lab` is a saturated colour that keeps its lightness.
fn kept(lab: [f32; 3]) -> f32 {
    smoothstep(KEEP_CHROMA.0, KEEP_CHROMA.1, lab[1].hypot(lab[2]))
        * smoothstep(KEEP_ABOVE.0, KEEP_ABOVE.1, lab[0])
}

/// `c` moved to `dark` lightness, or kept as far as it is saturated.
fn toward(c: Color, dark: impl Fn(f32) -> f32) -> Color {
    let lab @ [l, a, b] = lab_of(c);
    let keep = kept(lab);
    let k = FACE_CHROMA + (1.0 - FACE_CHROMA) * keep;
    color_of([dark(l) + (l - dark(l)) * keep, a * k, b * k])
}

/// A face colour of a part whose body runs over `r`.
pub(super) fn face_color(c: Color, r: Range) -> Color {
    let [o0, o1, o2, o3] = OUTLINE;
    let knots = [o0, o1, o2, o3, (BODY_FROM, r.0), (1.0, r.1)];
    toward(c, |l| curve(&knots, l))
}

/// A glyph, tick or dot drawn over a face.
pub(super) fn mark_color(c: Color) -> Color {
    toward(c, |l| curve(&INK, l))
}

fn leak(g: Vec<(f32, Color)>) -> Grad {
    Box::leak(g.into_boxed_slice())
}

fn grad(g: Grad, r: Range) -> Grad {
    leak(g.iter().map(|&(t, c)| (t, face_color(c, r))).collect())
}

/// `f` in the twin, its body over `r`.
pub(super) fn face(f: &Face, r: Range) -> Face {
    Face {
        rim: face_color(f.rim, r),
        inner: face_color(f.inner, r),
        halo: f.halo,
        down: grad(f.down, r),
        across: grad(f.across, r),
        mark: f.mark.map(mark_color),
    }
}

/// A tab's face in the twin: its `across` is a tint the body is multiplied by, so it is kept.
fn tinted(f: &Face, r: Range) -> Face {
    Face {
        across: f.across,
        ..face(f, r)
    }
}

/// A part's faces in the twin, each its body over `r`.
trait Darken {
    fn darken(&self, r: Range) -> Self;
}

/// Builds without `..`, so a state added to a type fails to build until it is listed.
macro_rules! darken {
    ($($t:ident { $($f:ident),* $(; $o:ident)? })*) => {$(
        impl Darken for $t {
            fn darken(&self, r: Range) -> Self {
                $t {
                    $($f: face(&self.$f, r),)*
                    $($o: self.$o.as_ref().map(|f| face(f, r)),)?
                }
            }
        }
    )*};
}

darken! {
    LitFaces { normal, hot, pressed }
    RestFaces { normal }
    PushButtonFaces { normal, hot, pressed, disabled, default, focused }
    CheckBoxFaces {
        normal, hot, pressed, disabled, normal_checked, hot_checked, pressed_checked,
        disabled_checked, normal_mixed, hot_mixed, pressed_mixed, disabled_mixed
    }
    RadioFaces {
        normal, hot, pressed, disabled, normal_checked, hot_checked, pressed_checked,
        disabled_checked
    }
    GreyedFaces { normal, disabled }
    LitGreyedFaces { normal, hot, pressed, disabled }
    ScrollGripFaces { normal, hot, pressed; disabled }
    ScrollTrackFaces { normal, pressed, disabled }
    ToolbarFaces { hot, pressed, checked, checked_hot }
    HeaderSortArrowFaces { normal, checked }
}

impl<T: Darken> Darken for Option<T> {
    fn darken(&self, r: Range) -> Self {
        self.as_ref().map(|t| t.darken(r))
    }
}

/// `s`'s dark twin: client inks and control faces dark, chrome as it is.
fn darken(s: &Scheme) -> Scheme {
    let sys = &s.system;
    let i = &s.inks;
    let h = hue(s.task_pane.normal.inner);
    let (c, x) = (&s.task_collapse, &s.task_expand);
    Scheme {
        system: System {
            button_face: ink(sys.button_face),
            button_hilight: ink(sys.button_hilight),
            button_light: ink(sys.button_light),
            button_shadow: ink(sys.button_shadow),
            button_dk_shadow: ink(sys.button_dk_shadow),
            button_text: ink(sys.button_text),
            window: ink(sys.window),
            window_text: ink(sys.window_text),
            gray_text: ink(sys.gray_text),
            menu: ink(sys.menu),
            menu_text: ink(sys.menu_text),
            info_bk: relit(ink(sys.info_bk), (INFO_BK, 1.0)),
            info_text: ink(sys.info_text),
            menu_bar: ink(sys.menu_bar),
            hot_light: ink(sys.hot_light),
            ..*sys
        },
        inks: Inks {
            greyed_label: ink(i.greyed_label),
            group_label: ink(i.group_label),
            toolbar_disabled: ink(i.toolbar_disabled),
            toolbar_drop_disabled: ink(i.toolbar_drop_disabled),
            rebar_label: ink(i.rebar_label),
            track_tick: ink(i.track_tick),
            task_title: tinted_at(h, TASK_INK),
            task_title_hot: tinted_at(h, TASK_INK_HOT),
            task_link: tinted_at(h, TASK_INK),
            task_link_hot: tinted_at(h, TASK_INK_HOT),
            ..*i
        },
        push_button: s.push_button.darken(RAISED),
        check_box: s.check_box.darken(WELL),
        radio: s.radio.darken(WELL),
        group_box: s.group_box.darken(LINE),
        scroll_arrow_up: s.scroll_arrow_up.darken(RAISED),
        scroll_arrow_down: s.scroll_arrow_down.darken(RAISED),
        scroll_arrow_left: s.scroll_arrow_left.darken(RAISED),
        scroll_arrow_right: s.scroll_arrow_right.darken(RAISED),
        scroll_thumb_v: s.scroll_thumb_v.darken(RAISED),
        scroll_thumb_h: s.scroll_thumb_h.darken(RAISED),
        scroll_grip_v: s.scroll_grip_v.darken(RAISED),
        scroll_grip_h: s.scroll_grip_h.darken(RAISED),
        scroll_track_v: s.scroll_track_v.darken(TRACK),
        scroll_track_h: s.scroll_track_h.darken(TRACK),
        edit_border: s.edit_border.darken(WELL),
        list_border: s.list_border.darken(WELL),
        combo_drop: s.combo_drop.darken(RAISED),
        tab_item: TabItemFaces {
            normal: tinted(&s.tab_item.normal, PANE),
            hot: tinted(&s.tab_item.hot, PANE),
            checked: tinted(&s.tab_item.checked, PANE),
        },
        tab_pane: s.tab_pane.darken(PANE),
        progress_bar: s.progress_bar.darken(WELL),
        progress_chunk: s.progress_chunk.darken(RAISED),
        trackbar_track: s.trackbar_track.darken(TRACK),
        trackbar_thumb: s.trackbar_thumb.darken(RAISED),
        trackbar_track_v: s.trackbar_track_v.darken(TRACK),
        trackbar_thumb_v: s.trackbar_thumb_v.darken(RAISED),
        spin_up: s.spin_up.darken(RAISED),
        spin_down: s.spin_down.darken(RAISED),
        size_box: s.size_box.darken(PANE),
        toolbar_button: s.toolbar_button.darken(RAISED),
        toolbar_drop: s.toolbar_drop.darken(RAISED),
        toolbar_separator: s.toolbar_separator.darken(PANE),
        rebar: s.rebar.darken(PANE),
        rebar_band: s.rebar_band.darken(PANE),
        header_item: s.header_item.darken(RAISED),
        header_filler: s.header_filler.darken(PANE),
        header_sort_arrow: s.header_sort_arrow.darken(RAISED),
        status_bar: s.status_bar.darken(PANE),
        status_pane: s.status_pane.darken(PANE),
        explorer_bar_header: s.explorer_bar_header.darken(PANE),
        explorer_bar_close: s.explorer_bar_close.darken(RAISED),
        task_pane: RestFaces {
            normal: deep_face(&s.task_pane.normal, h),
        },
        task_group_header: RestFaces {
            normal: deep_face(&s.task_group_header.normal, h),
        },
        task_group_body: RestFaces {
            normal: deep_face(&s.task_group_body.normal, h),
        },
        task_collapse: LitFaces {
            normal: disc(&c.normal, h, TASK_INK),
            hot: disc(&c.hot, h, TASK_INK_HOT),
            pressed: disc(&c.pressed, h, TASK_INK_HOT),
        },
        task_expand: RestFaces {
            normal: disc(&x.normal, h, TASK_INK),
        },
        places_bar: s.places_bar.darken(SUNK),
        start_menu: StartMenu {
            rule: ink(s.start_menu.rule),
            ..s.start_menu
        },
        welcome_band: relit(s.welcome_band, WELCOME_BAND),
        welcome_field: relit(s.welcome_field, WELCOME_FIELD),
        welcome_glow: relit(s.welcome_glow, WELCOME_GLOW),
        welcome_tile_hot: relit(s.welcome_tile_hot, WELCOME_TILE_HOT),
        ..*s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn l(c: Color) -> f32 {
        lab_of(c)[0]
    }

    fn navy() -> &'static Scheme {
        twin(Variant::Blue)
    }

    #[test]
    fn a_twin_is_built_once_and_a_variant_without_one_keeps_its_light_scheme() {
        assert!(std::ptr::eq(navy(), twin(Variant::Blue)));
        assert_eq!(twin(Variant::Olive), &OLIVE);
        assert_ne!(navy().system, NAVY.system);
    }

    #[test]
    fn client_surfaces_go_charcoal_and_their_text_light() {
        let s = &navy().system;
        for (what, c) in [
            ("window", s.window),
            ("button_face", s.button_face),
            ("menu", s.menu),
            ("menu_bar", s.menu_bar),
        ] {
            assert!((0.2..0.3).contains(&l(c)), "{what} is charcoal, got {c:?}");
        }
        for (what, c) in [
            ("window_text", s.window_text),
            ("button_text", s.button_text),
            ("menu_text", s.menu_text),
            ("info_text", s.info_text),
        ] {
            assert!(l(c) > 0.85, "{what} is light, got {c:?}");
        }
        assert!(
            l(s.window) < l(s.button_face),
            "a list is darker than its dialog"
        );
        let grey = l(s.gray_text);
        assert!(grey > l(s.button_face) + 0.25 && grey < l(s.button_text) - 0.2);
        assert!(
            l(s.button_shadow) > l(s.button_face) + 0.25,
            "a shadow line reads on the face"
        );
    }

    #[test]
    fn chrome_selection_and_the_desktop_keep_the_light_colours() {
        let (light, dark) = (&NAVY, navy());
        assert_eq!(dark.caption, light.caption);
        assert_eq!(dark.max_caption, light.max_caption);
        assert_eq!(dark.frame_left, light.frame_left);
        assert_eq!(dark.frame_right, light.frame_right);
        assert_eq!(dark.frame_bottom, light.frame_bottom);
        assert_eq!(dark.close_button, light.close_button);
        assert_eq!(dark.min_button, light.min_button);
        assert_eq!(dark.taskbar, light.taskbar);
        assert_eq!(dark.start_button, light.start_button);
        assert_eq!(dark.task_button, light.task_button);
        assert_eq!(dark.tray_area, light.tray_area);
        assert_eq!(dark.metrics, light.metrics);
        let (ls, ds) = (&light.system, &dark.system);
        for (what, a, b) in [
            ("hilight", ls.hilight, ds.hilight),
            ("hilight_text", ls.hilight_text, ds.hilight_text),
            ("menu_hilight", ls.menu_hilight, ds.menu_hilight),
            ("active_title", ls.active_title, ds.active_title),
            ("background", ls.background, ds.background),
        ] {
            assert_eq!(a, b, "{what} is kept");
        }
        assert_eq!(dark.inks.title, light.inks.title);
        assert_eq!(dark.inks.task_normal, light.inks.task_normal);
    }

    /// Each colour of `dark` against the one at the same stop of `light`.
    fn pairs<'a>(light: &'a Face, dark: &'a Face) -> impl Iterator<Item = (Color, Color)> + 'a {
        let g = |a: Grad, b: Grad| a.iter().zip(b).map(|(x, y)| (x.1, y.1));
        g(light.down, dark.down).chain(g(light.across, dark.across))
    }

    fn near(a: Color, b: Color) -> bool {
        [(a.r, b.r), (a.g, b.g), (a.b, b.b)]
            .iter()
            .all(|&(x, y)| x.abs_diff(y) <= 2)
    }

    #[test]
    fn the_hover_rim_focus_ring_and_check_marks_keep_their_colour() {
        let (light, dark) = (&NAVY, navy());
        let orange = Color::rgb(0xF8, 0xBB, 0x2B);
        let blue = Color::rgb(0x65, 0x78, 0xEE);
        for (lf, df, accent) in [
            (&light.push_button.hot, &dark.push_button.hot, orange),
            (&light.push_button.default, &dark.push_button.default, blue),
            (&light.push_button.focused, &dark.push_button.focused, blue),
        ] {
            let (_, got) = pairs(lf, df)
                .find(|&(a, _)| a == accent)
                .expect("the light face carries the accent");
            assert!(near(got, accent), "{accent:?} is kept, got {got:?}");
        }
        for (a, b) in [
            (
                light.check_box.normal_checked.mark,
                dark.check_box.normal_checked.mark,
            ),
            (
                light.radio.normal_checked.mark,
                dark.radio.normal_checked.mark,
            ),
            (light.tab_item.checked.mark, dark.tab_item.checked.mark),
        ] {
            let (a, b) = (a.unwrap(), b.unwrap());
            assert!(near(a, b), "{a:?} is kept, got {b:?}");
        }
    }

    #[test]
    fn a_body_keeps_its_gloss_direction() {
        let (light, dark) = (&NAVY, navy());
        let p = (&light.push_button, &dark.push_button);
        let t = (&light.scroll_thumb_v, &dark.scroll_thumb_v);
        for (lf, df) in [
            (&p.0.normal, &p.1.normal),
            (&p.0.pressed, &p.1.pressed),
            (&t.0.normal, &t.1.normal),
            (&t.0.hot, &t.1.hot),
            (&light.tab_item.normal, &dark.tab_item.normal),
        ] {
            let body: Vec<_> = pairs(lf, df)
                .filter(|&(a, _)| l(a) >= BODY_FROM && kept(lab_of(a)) == 0.0)
                .collect();
            for w in body.windows(2) {
                let (da, db) = (l(w[1].0) - l(w[0].0), l(w[1].1) - l(w[0].1));
                assert!(da.abs() < 0.01 || da * db >= 0.0, "{w:?} turns back");
            }
        }
    }

    #[test]
    fn outlines_read_on_charcoal_and_tracks_sit_under_their_thumbs() {
        let d = navy();
        let face = l(d.system.button_face);
        for (what, rim) in [
            ("push button", d.push_button.normal.rim),
            ("check box", d.check_box.normal.rim),
            ("radio", d.radio.normal.rim),
            ("edit", d.edit_border.normal.rim),
            ("group box", d.group_box.normal.rim),
            ("tab page", d.tab_pane.normal.rim),
        ] {
            assert!(
                l(rim) > face + 0.12,
                "the {what} outline is lighter than the dialog"
            );
        }
        let track = l(d.scroll_track_v.normal.across[1].1);
        let thumb = l(d.scroll_thumb_v.normal.inner);
        assert!(track < thumb, "track {track} under thumb {thumb}");
        assert!(track < face, "the track is sunk into the dialog");
    }

    /// The OKLab hue angle of `c`, in radians.
    fn angle(c: Color) -> f32 {
        let [_, a, b] = lab_of(c);
        b.atan2(a)
    }

    #[test]
    fn task_panes_go_deep_in_the_panes_hue_under_light_titles_and_links() {
        let (light, d) = (&NAVY, navy());
        let pane = d.task_pane.normal.down;
        let header = &d.task_group_header.normal;
        let body = &d.task_group_body.normal;
        let hue = angle(light.task_pane.normal.inner);
        let (top, bottom) = (pane[0].1, pane[pane.len() - 1].1);
        assert!(l(top) > l(bottom), "the pane shades down");
        for c in [top, bottom, header.across[0].1, body.inner] {
            assert!(l(c) < 0.36, "{c:?} is deep");
            assert!((angle(c) - hue).abs() < 0.05, "{c:?} keeps the pane's hue");
        }
        assert!(
            l(header.across[0].1) > l(top),
            "a title band stands above the pane"
        );
        assert!(
            l(body.inner) > l(top),
            "a group's body stands above the pane"
        );
        let i = &d.inks;
        for bg in [header.across[0].1, header.across[2].1, body.inner] {
            for (what, ink) in [("title", i.task_title), ("link", i.task_link)] {
                assert!(ink.contrast(bg) >= 4.5, "the {what} reads on {bg:?}");
            }
        }
        assert!(l(i.task_link_hot) > l(i.task_link), "a lit link brightens");
        let c = &d.task_collapse;
        assert!(
            l(c.normal.inner) > l(header.across[0].1),
            "the disc stands on its band"
        );
        assert!(l(c.normal.rim) > l(c.normal.inner), "the disc's rim is lit");
        assert_eq!(c.normal.mark, Some(i.task_title));
        assert_eq!(c.hot.mark, Some(i.task_title_hot));
    }

    #[test]
    fn the_places_bar_sinks_like_a_list_and_the_start_menu_keeps_its_places_column() {
        let (light, d) = (&NAVY, navy());
        let face = l(d.system.button_face);
        let p = &d.places_bar.normal;
        for c in [p.rim, p.inner, p.down[2].1] {
            assert!(l(c) < face, "{c:?} sits under the dialog");
        }
        assert!(
            d.system.window_text.contrast(p.inner) >= 4.5,
            "a place's label reads"
        );
        let (lm, dm) = (&light.start_menu, &d.start_menu);
        assert_eq!(
            (dm.places, dm.places_ink, dm.places_edge),
            (lm.places, lm.places_ink, lm.places_edge)
        );
        assert_eq!((dm.band, dm.edge, dm.accent), (lm.band, lm.edge, lm.accent));
        let rule = l(dm.rule);
        assert!(
            rule > l(d.system.window) && rule < l(d.system.gray_text),
            "a faint rule"
        );
    }

    #[test]
    fn tips_stand_above_the_window_and_the_welcome_screen_goes_deeper() {
        let (light, d) = (&NAVY, navy());
        let s = &d.system;
        assert!(
            l(s.info_bk) > l(s.window) + 0.05,
            "a tip stands off the window"
        );
        assert!(s.info_text.contrast(s.info_bk) >= 7.0);
        for (what, a, b) in [
            ("band", light.welcome_band, d.welcome_band),
            ("field", light.welcome_field, d.welcome_field),
            ("glow", light.welcome_glow, d.welcome_glow),
            ("hot tile", light.welcome_tile_hot, d.welcome_tile_hot),
        ] {
            assert!(l(b) < l(a), "the {what} is deeper");
            assert!(
                (angle(a) - angle(b)).abs() < 0.05,
                "the {what} keeps its hue"
            );
            assert!(
                d.welcome_text.contrast(b) >= 4.5,
                "text reads on the {what}"
            );
        }
        assert!(
            l(d.welcome_glow) > l(d.welcome_field),
            "the glow still lights the field"
        );
        assert_eq!(
            (d.welcome_text, d.welcome_rule, d.welcome_tile_frame),
            (
                light.welcome_text,
                light.welcome_rule,
                light.welcome_tile_frame
            )
        );
    }
}
