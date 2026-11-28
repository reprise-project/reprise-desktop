//! Dark twins: a Blue scheme at night, built from its light scheme by three colour rules in
//! OKLab and a lightness range per part, then a few parts set by hand.
//! - Chrome ([`night`]): captions, frames, caption buttons, the taskbar, Start, task buttons, the
//!   tray and the Start menu's bands keep their gradients and hue at a lower lightness
//!   ([`NIGHT`]) and chroma ([`NIGHT_CHROMA`]). An inactive window's chrome sinks further
//!   ([`INACTIVE`]). Glyphs, title text and labels are kept.
//! - Client inks ([`ink`]): lightness through [`INK`], so white goes a deep ground and black
//!   light, and every ink carries the scheme's hue ([`TINT`]), so the ground is navy in navy and
//!   olive in olive. Menus and tips stand above the window ([`MENU`], [`INFO_BK`]). Selection and
//!   desktop colours are kept.
//! - Control faces ([`face`]): a light body (lightness from [`BODY_FROM`] up) lands on its
//!   part's [`Range`] in order, so gloss keeps its direction, tinted like the inks; darker
//!   outlines turn light through [`OUTLINE`] and marks through [`INK`]. Saturated colours (the
//!   hover rim, the focus and default rings, check marks, the progress fill) keep their
//!   lightness. A disabled face goes flat near the dialog under a dim rim ([`greyed`]), and a
//!   held arrow button turns a lit blue ([`held`]).
//! - Task panes ([`deep`]): every colour takes the light pane's hue at one low chroma, its
//!   lightness through [`DEEP`], under light titles and links; fold discs stand lighter.
//! - The Start menu's program column follows the window ink, its places column stands above it
//!   ([`PLACES`]). The welcome screen's blues go deeper ([`relit`]).

use std::sync::OnceLock;

use super::*;
use crate::blue::trayarrow::ArrowInks;
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

/// Light to dark lightness for chrome, linear between knots: the active caption (0.48) lands
/// near 0.35, its gloss band (0.64) near 0.47, and a white rim at 0.70.
const NIGHT: [(f32, f32); 4] = [(0.00, 0.00), (0.45, 0.33), (0.65, 0.47), (1.00, 0.70)];

/// How much of a chrome colour's OKLab chroma its night colour keeps.
const NIGHT_CHROMA: f32 = 0.6;

/// An inactive window's chrome after [`NIGHT`]: (share of lightness, share of chroma) kept, so
/// it sinks towards the windows' ground and greys.
const INACTIVE: (f32, f32) = (0.72, 0.5);

/// Light to dark lightness for client inks, linear between knots: text (0) goes light, the
/// dialog face (0.93) and window (1) a deep ground, grey text (0.73) a mid grey.
const INK: [(f32, f32); 7] = [
    (0.00, 0.92),
    (0.30, 0.82),
    (0.50, 0.74),
    (0.73, 0.62),
    (0.82, 0.33),
    (0.93, 0.25),
    (1.00, 0.20),
];

/// How much of a light ink's OKLab chroma its dark ink keeps.
const INK_CHROMA: f32 = 0.3;

/// OKLab chroma in the scheme's hue added to a dark ink or face colour, by its lightness: the
/// ground is tinted most, light text barely.
const TINT: [(f32, f32); 4] = [(0.15, 0.034), (0.30, 0.032), (0.60, 0.012), (0.95, 0.006)];

/// The scheme's caption chroma from which [`TINT`] is whole; a greyer scheme is tinted less.
const HUE_FULL: f32 = 0.06;

/// The lightness of a menu, above the window and dialog it drops over.
const MENU: f32 = 0.29;

/// A tooltip's lightness: above the windows it pops over.
const INFO_BK: f32 = 0.31;

/// The Start menu's places column: above the program column.
const PLACES: f32 = 0.30;

/// Lightness from which a face colour is part of the body; below it, an outline or glyph.
const BODY_FROM: f32 = 0.78;

/// Outline lightness, linear between knots up to 0.70, then on to the body's range at
/// [`BODY_FROM`]: dark rims turn light, so a control's outline reads on the dark ground.
const OUTLINE: [(f32, f32); 4] = [(0.00, 0.93), (0.35, 0.66), (0.55, 0.60), (0.70, 0.54)];

/// How much OKLab chroma a mapped face colour keeps.
const FACE_CHROMA: f32 = 0.8;

/// Chroma, then lightness, over which a face colour moves from mapped to kept.
const KEEP_CHROMA: (f32, f32) = (0.085, 0.115);
const KEEP_ABOVE: (f32, f32) = (0.42, 0.52);

/// The lightness a part's body runs over in the twin, from its darkest to its lightest colour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Range(pub f32, pub f32);

/// Buttons, arrows, thumbs and headers: a glossy face lighter than the dialog.
const RAISED: Range = Range(0.31, 0.48);
/// Check box and radio wells, the progress trough and edit borders.
const WELL: Range = Range(0.20, 0.30);
/// Scrollbar and trackbar tracks, darker than their thumbs.
const TRACK: Range = Range(0.18, 0.23);
/// Tabs and their pages (an open tab is its page's colour), status bars, rebars, the explorer
/// bar's header and the list header's filler, near the dialog face.
const PANE: Range = Range(0.25, 0.31);
/// A group box's line, lighter than the dialog it divides.
const LINE: Range = Range(0.42, 0.46);
/// The file dialog's places bar, sunk below the dialog like a list.
const SUNK: Range = Range(0.19, 0.23);

/// A held arrow button: its light face from [`HELD_FROM`] up runs over this range, none kept,
/// so the pressed lavender turns a lit blue a step above the raised face.
const HELD: Range = Range(0.34, 0.50);
const HELD_FROM: f32 = 0.6;

/// A disabled face: its body flat near the dialog, its rim and marks dim, at a share of chroma.
const GREYED: Range = Range(0.27, 0.29);
const GREYED_RIM: f32 = 0.38;
const GREYED_MARK: f32 = 0.52;
const GREYED_CHROMA: f32 = 0.3;

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

/// The scheme's tint: OKLab (a, b) per unit of [`TINT`], the hue of its active caption, shorter
/// for a caption greyer than [`HUE_FULL`].
#[derive(Clone, Copy)]
struct Tint([f32; 2]);

impl Tint {
    fn of(s: &Scheme) -> Tint {
        let [_, a, b] = lab_of(s.system.active_title);
        let n = a.hypot(b).max(HUE_FULL);
        Tint([a / n, b / n])
    }

    /// The (a, b) added to a colour at lightness `l`, scaled by `k`.
    fn at(self, l: f32, k: f32) -> [f32; 2] {
        let c = curve(&TINT, l) * k;
        [self.0[0] * c, self.0[1] * c]
    }
}

/// `c` at night: lightness through [`NIGHT`], chroma scaled by [`NIGHT_CHROMA`], and for an
/// inactive window's chrome by [`INACTIVE`] as well.
fn night(c: Color, inactive: bool) -> Color {
    let [l, a, b] = lab_of(c);
    let (lk, ck) = if inactive { INACTIVE } else { (1.0, 1.0) };
    let k = NIGHT_CHROMA * ck;
    color_of([curve(&NIGHT, l) * lk, a * k, b * k])
}

fn night_grad(g: Grad, inactive: bool) -> Grad {
    leak(g.iter().map(|&(t, c)| (t, night(c, inactive))).collect())
}

/// A chrome face at night; its glyph is kept.
fn night_face(f: &Face, inactive: bool) -> Face {
    Face {
        rim: night(f.rim, inactive),
        inner: night(f.inner, inactive),
        halo: f.halo.map(|(c, a)| (night(c, inactive), a)),
        down: night_grad(f.down, inactive),
        across: night_grad(f.across, inactive),
        mark: f.mark,
    }
}

/// A part's chrome faces at night.
trait Night {
    fn night(&self) -> Self;
}

/// Builds without `..`, so a state added to a type fails to build until it is listed.
macro_rules! night {
    ($($t:ident { $($f:ident),* $(; inactive $i:ident)? })*) => {$(
        impl Night for $t {
            fn night(&self) -> Self {
                $t {
                    $($f: night_face(&self.$f, false),)*
                    $($i: night_face(&self.$i, true),)?
                }
            }
        }
    )*};
}

night! {
    FrameFaces { normal; inactive inactive }
    CaptionButtonFaces { normal, hot, pressed, disabled; inactive inactive }
    HelpButtonFaces { normal, hot, pressed; inactive inactive }
    TaskButtonFaces { normal, hot, checked, checked_hot, pressed }
}

fn night_lit(f: &LitFaces) -> LitFaces {
    LitFaces {
        normal: night_face(&f.normal, false),
        hot: night_face(&f.hot, false),
        pressed: night_face(&f.pressed, false),
    }
}

/// The tray's arrow at night; its glyph is kept.
fn night_arrow(r: &ArrowRecipe) -> ArrowRecipe {
    let n = |c| night(c, false);
    let inks = |i: &ArrowInks| ArrowInks {
        face: i.face.map(n),
        gloss: n(i.gloss),
        glow: n(i.glow),
        ring: n(i.ring),
        edge: n(i.edge),
        ..*i
    };
    ArrowRecipe {
        rim: r.rim.map(n),
        states: r.states.each_ref().map(inks),
        ..*r
    }
}

/// `c` as a dark client ink, tinted by `t`.
fn ink(c: Color, t: Tint) -> Color {
    let [l, a, b] = lab_of(c);
    let l = curve(&INK, l);
    let [ta, tb] = t.at(l, 1.0);
    color_of([l, a * INK_CHROMA + ta, b * INK_CHROMA + tb])
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

/// `c` moved to `dark` lightness and tinted by `t`, or kept as far as it is saturated.
fn toward(c: Color, t: Tint, dark: impl Fn(f32) -> f32) -> Color {
    let lab @ [l, a, b] = lab_of(c);
    let keep = kept(lab);
    let k = FACE_CHROMA + (1.0 - FACE_CHROMA) * keep;
    let l = dark(l) + (l - dark(l)) * keep;
    let [ta, tb] = t.at(l, 1.0 - keep);
    color_of([l, a * k + ta, b * k + tb])
}

/// A part's colours in the twin: its scheme's tint and its body's range.
#[derive(Clone, Copy)]
struct Part(Tint, Range);

/// A face colour of a part.
fn face_color(c: Color, Part(t, r): Part) -> Color {
    let [o0, o1, o2, o3] = OUTLINE;
    let knots = [o0, o1, o2, o3, (BODY_FROM, r.0), (1.0, r.1)];
    toward(c, t, |l| curve(&knots, l))
}

/// A glyph, tick or dot drawn over a face.
fn mark_color(c: Color, t: Tint) -> Color {
    toward(c, t, |l| curve(&INK, l))
}

fn leak(g: Vec<(f32, Color)>) -> Grad {
    Box::leak(g.into_boxed_slice())
}

fn map_face(f: &Face, c: impl Fn(Color) -> Color, mark: impl Fn(Color) -> Color) -> Face {
    let grad = |g: Grad| leak(g.iter().map(|&(t, x)| (t, c(x))).collect());
    Face {
        rim: c(f.rim),
        inner: c(f.inner),
        halo: f.halo,
        down: grad(f.down),
        across: grad(f.across),
        mark: f.mark.map(mark),
    }
}

/// `f` in the twin as part `p`.
fn face(f: &Face, p: Part) -> Face {
    map_face(f, |c| face_color(c, p), |c| mark_color(c, p.0))
}

/// `c` greyed at lightness `l`: [`GREYED_CHROMA`] of its chroma, tinted by `t`.
fn grey(c: Color, l: f32, t: Tint) -> Color {
    let [_, a, b] = lab_of(c);
    let [ta, tb] = t.at(l, 1.0);
    color_of([l, a * GREYED_CHROMA + ta, b * GREYED_CHROMA + tb])
}

/// A disabled face: its body (the light face's colours from [`BODY_FROM`] up, other than its
/// rim) flat over [`GREYED`], its rim and outlines at [`GREYED_RIM`], its mark at
/// [`GREYED_MARK`], all at [`GREYED_CHROMA`] and tinted.
fn greyed(f: &Face, t: Tint) -> Face {
    let at = |c: Color, l: f32| grey(c, l, t);
    let body = |c: Color| {
        let l = lab_of(c)[0];
        if c == f.rim || l < BODY_FROM {
            at(c, GREYED_RIM)
        } else {
            at(
                c,
                GREYED.0 + (GREYED.1 - GREYED.0) * (l - BODY_FROM) / (1.0 - BODY_FROM),
            )
        }
    };
    map_face(f, body, |c| at(c, GREYED_MARK))
}

/// A held arrow's face: every colour on [`HELD`] by its lightness, at [`FACE_CHROMA`], tinted.
fn held(f: &Face, t: Tint) -> Face {
    let body = |c: Color| {
        let [l, a, b] = lab_of(c);
        let k = ((l - HELD_FROM) / (1.0 - HELD_FROM)).clamp(0.0, 1.0);
        let l = HELD.0 + (HELD.1 - HELD.0) * k;
        let [ta, tb] = t.at(l, 1.0);
        color_of([l, a * FACE_CHROMA + ta, b * FACE_CHROMA + tb])
    };
    map_face(f, body, |c| mark_color(c, t))
}

/// An arrow button in the twin: raised, [`held`] when pressed, its disabled glyph set.
fn arrow(f: &LitGreyedFaces, t: Tint) -> LitGreyedFaces {
    let d = greyed_glyph(f.darken(Part(t, RAISED)), t);
    LitGreyedFaces {
        pressed: held(&f.pressed, t),
        ..d
    }
}

/// `f` with its disabled glyph set: the painter shades a disabled arrow from the face, which
/// would vanish on the dark one.
fn greyed_glyph(f: LitGreyedFaces, t: Tint) -> LitGreyedFaces {
    let glyph = grey(f.disabled.inner, GREYED_MARK, t);
    LitGreyedFaces {
        disabled: Face {
            mark: f.disabled.mark.or(Some(glyph)),
            ..f.disabled
        },
        ..f
    }
}

/// A tab's face in the twin: its `across` is a tint the body is multiplied by, so it is kept.
fn tab(f: &Face, p: Part) -> Face {
    Face {
        across: f.across,
        ..face(f, p)
    }
}

/// A part's faces in the twin: lit states as part `p`, disabled ones [`greyed`] unless they
/// repeat the normal face.
trait Darken {
    fn darken(&self, p: Part) -> Self;
}

/// Builds without `..`, so a state added to a type fails to build until it is listed.
macro_rules! darken {
    ($($t:ident { $($f:ident),* $(; greyed $($g:ident),*)? $(; maybe $o:ident)? })*) => {$(
        impl Darken for $t {
            fn darken(&self, p: Part) -> Self {
                $t {
                    $($f: face(&self.$f, p),)*
                    $($($g: if self.$g == self.normal {
                        face(&self.$g, p)
                    } else {
                        greyed(&self.$g, p.0)
                    },)*)?
                    $($o: self.$o.as_ref().map(|f| greyed(f, p.0)),)?
                }
            }
        }
    )*};
}

darken! {
    LitFaces { normal, hot, pressed }
    RestFaces { normal }
    PushButtonFaces { normal, hot, pressed, default, focused; greyed disabled }
    CheckBoxFaces {
        normal, hot, pressed, normal_checked, hot_checked, pressed_checked, normal_mixed,
        hot_mixed, pressed_mixed; greyed disabled, disabled_checked, disabled_mixed
    }
    RadioFaces {
        normal, hot, pressed, normal_checked, hot_checked, pressed_checked;
        greyed disabled, disabled_checked
    }
    GreyedFaces { normal; greyed disabled }
    LitGreyedFaces { normal, hot, pressed; greyed disabled }
    ScrollGripFaces { normal, hot, pressed; maybe disabled }
    ScrollTrackFaces { normal, pressed; greyed disabled }
    ToolbarFaces { hot, pressed, checked, checked_hot }
    HeaderSortArrowFaces { normal, checked }
}

impl<T: Darken> Darken for Option<T> {
    fn darken(&self, p: Part) -> Self {
        self.as_ref().map(|t| t.darken(p))
    }
}

/// `s`'s dark twin: chrome at night, client inks and control faces dark and tinted.
fn darken(s: &Scheme) -> Scheme {
    let sys = &s.system;
    let i = &s.inks;
    let t = Tint::of(s);
    let p = |r| Part(t, r);
    let ink = |c| ink(c, t);
    let n = |c| night(c, false);
    let h = hue(s.task_pane.normal.inner);
    let (c, x) = (&s.task_collapse, &s.task_expand);
    let m = &s.start_menu;
    Scheme {
        system: System {
            active_title: n(sys.active_title),
            gradient_active_title: n(sys.gradient_active_title),
            inactive_title: night(sys.inactive_title, true),
            gradient_inactive_title: night(sys.gradient_inactive_title, true),
            button_face: ink(sys.button_face),
            button_hilight: ink(sys.button_hilight),
            button_light: ink(sys.button_light),
            button_shadow: ink(sys.button_shadow),
            button_dk_shadow: ink(sys.button_dk_shadow),
            button_text: ink(sys.button_text),
            window: ink(sys.window),
            window_text: ink(sys.window_text),
            gray_text: ink(sys.gray_text),
            menu: relit(ink(sys.menu), (MENU, 1.0)),
            menu_text: ink(sys.menu_text),
            info_bk: relit(ink(sys.info_bk), (INFO_BK, 1.0)),
            info_text: ink(sys.info_text),
            menu_bar: ink(sys.menu_bar),
            hot_light: ink(sys.hot_light),
            menu_band: n(sys.menu_band),
            gradient_menu_band: n(sys.gradient_menu_band),
            menu_band_top: n(sys.menu_band_top),
            ..*sys
        },
        inks: Inks {
            title_shadow: n(i.title_shadow),
            start_shadow: n(i.start_shadow),
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
        caption: s.caption.night(),
        max_caption: s.max_caption.night(),
        frame_left: s.frame_left.night(),
        frame_right: s.frame_right.night(),
        frame_bottom: s.frame_bottom.night(),
        min_button: s.min_button.night(),
        max_button: s.max_button.night(),
        restore_button: s.restore_button.night(),
        close_button: s.close_button.night(),
        help_button: s.help_button.night(),
        taskbar: RestFaces {
            normal: night_face(&s.taskbar.normal, false),
        },
        start_button: night_lit(&s.start_button),
        task_button: s.task_button.night(),
        tray_area: RestFaces {
            normal: night_face(&s.tray_area.normal, false),
        },
        tray_arrow: night_arrow(&s.tray_arrow),
        calendar_band: GreyedFaces {
            normal: night_face(&s.calendar_band.normal, false),
            disabled: night_face(&s.calendar_band.disabled, true),
        },
        push_button: s.push_button.darken(p(RAISED)),
        check_box: s.check_box.darken(p(WELL)),
        radio: s.radio.darken(p(WELL)),
        group_box: s.group_box.darken(p(LINE)),
        scroll_arrow_up: arrow(&s.scroll_arrow_up, t),
        scroll_arrow_down: arrow(&s.scroll_arrow_down, t),
        scroll_arrow_left: arrow(&s.scroll_arrow_left, t),
        scroll_arrow_right: arrow(&s.scroll_arrow_right, t),
        scroll_thumb_v: s.scroll_thumb_v.darken(p(RAISED)),
        scroll_thumb_h: s.scroll_thumb_h.darken(p(RAISED)),
        scroll_grip_v: s.scroll_grip_v.darken(p(RAISED)),
        scroll_grip_h: s.scroll_grip_h.darken(p(RAISED)),
        scroll_track_v: s.scroll_track_v.darken(p(TRACK)),
        scroll_track_h: s.scroll_track_h.darken(p(TRACK)),
        edit_border: s.edit_border.darken(p(WELL)),
        list_border: s.list_border.darken(p(WELL)),
        combo_drop: arrow(&s.combo_drop, t),
        tab_item: TabItemFaces {
            normal: tab(&s.tab_item.normal, p(PANE)),
            hot: tab(&s.tab_item.hot, p(PANE)),
            checked: tab(&s.tab_item.checked, p(PANE)),
        },
        tab_pane: s.tab_pane.darken(p(PANE)),
        progress_bar: s.progress_bar.darken(p(WELL)),
        progress_chunk: s.progress_chunk.darken(p(RAISED)),
        trackbar_track: s.trackbar_track.darken(p(TRACK)),
        trackbar_thumb: s.trackbar_thumb.darken(p(RAISED)),
        trackbar_track_v: s.trackbar_track_v.darken(p(TRACK)),
        trackbar_thumb_v: s.trackbar_thumb_v.darken(p(RAISED)),
        spin_up: arrow(&s.spin_up, t),
        spin_down: arrow(&s.spin_down, t),
        size_box: s.size_box.darken(p(PANE)),
        toolbar_button: s.toolbar_button.darken(p(RAISED)),
        toolbar_drop: s.toolbar_drop.darken(p(RAISED)),
        toolbar_separator: s.toolbar_separator.darken(p(PANE)),
        rebar: s.rebar.darken(p(PANE)),
        rebar_band: s.rebar_band.darken(p(PANE)),
        header_item: s.header_item.darken(p(RAISED)),
        header_filler: s.header_filler.darken(p(PANE)),
        header_sort_arrow: s.header_sort_arrow.darken(p(RAISED)),
        status_bar: s.status_bar.darken(p(PANE)),
        status_pane: s.status_pane.darken(p(PANE)),
        explorer_bar_header: s.explorer_bar_header.darken(p(PANE)),
        explorer_bar_close: s.explorer_bar_close.darken(p(RAISED)),
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
        places_bar: s.places_bar.darken(p(SUNK)),
        start_menu: StartMenu {
            band: night_grad(m.band, false),
            edge: n(m.edge),
            accent: n(m.accent),
            band_shadow: n(m.band_shadow),
            places: relit(ink(m.places), (PLACES, 1.0)),
            places_edge: ink(m.places_edge),
            places_ink: ink(m.places_ink),
            rule: ink(m.rule),
            ..*m
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

    /// The OKLab chroma of `c`.
    fn chroma(c: Color) -> f32 {
        let [_, a, b] = lab_of(c);
        a.hypot(b)
    }

    #[test]
    fn client_surfaces_go_deep_in_the_schemes_hue_and_their_text_light() {
        let s = &navy().system;
        let hue = angle(NAVY.system.active_title);
        for (what, c) in [
            ("window", s.window),
            ("button_face", s.button_face),
            ("menu", s.menu),
            ("menu_bar", s.menu_bar),
        ] {
            assert!((0.18..0.3).contains(&l(c)), "{what} is deep, got {c:?}");
            assert!(chroma(c) > 0.025, "{what} is tinted, got {c:?}");
            assert!((angle(c) - hue).abs() < 0.3, "{what} is navy, got {c:?}");
        }
        for (what, c) in [
            ("window_text", s.window_text),
            ("button_text", s.button_text),
            ("menu_text", s.menu_text),
            ("info_text", s.info_text),
        ] {
            assert!(l(c) > 0.85, "{what} is light, got {c:?}");
            assert!(chroma(c) < 0.02, "{what} is near white, got {c:?}");
        }
        assert!(
            l(s.window) < l(s.button_face),
            "a list is darker than its dialog"
        );
        assert!(
            l(s.menu) > l(s.button_face),
            "a menu stands above the dialog"
        );
        let grey = l(s.gray_text);
        assert!(grey > l(s.button_face) + 0.25 && grey < l(s.button_text) - 0.2);
        assert!(
            l(s.button_shadow) > l(s.button_face) + 0.25,
            "a shadow line reads on the face"
        );
    }

    #[test]
    fn chrome_goes_night_in_its_own_hue_under_the_same_titles() {
        let (light, dark) = (&NAVY, navy());
        for (what, a, b) in [
            ("caption", &light.caption.normal, &dark.caption.normal),
            ("frame", &light.frame_left.normal, &dark.frame_left.normal),
            ("taskbar", &light.taskbar.normal, &dark.taskbar.normal),
            (
                "task button",
                &light.task_button.normal,
                &dark.task_button.normal,
            ),
            ("tray", &light.tray_area.normal, &dark.tray_area.normal),
            (
                "Start",
                &light.start_button.normal,
                &dark.start_button.normal,
            ),
            (
                "close",
                &light.close_button.normal,
                &dark.close_button.normal,
            ),
        ] {
            for (x, y) in pairs(a, b).chain([(a.inner, b.inner)]) {
                assert!(l(y) < l(x), "the {what}'s {x:?} goes deeper, got {y:?}");
                assert!(
                    chroma(y) <= chroma(x) + 1e-4,
                    "the {what}'s {x:?} greys, got {y:?}"
                );
                if chroma(x) > 0.08 {
                    assert!(
                        chroma(y) > 0.04,
                        "the {what}'s {x:?} keeps colour, got {y:?}"
                    );
                    let turn = (angle(x) - angle(y)).abs();
                    assert!(turn < 0.05, "the {what}'s {x:?} keeps its hue, got {y:?}");
                }
            }
            assert_eq!(a.mark, b.mark, "the {what}'s glyph is kept");
        }
        let body = |f: &Face| f.down[f.down.len() / 2].1;
        let (on, off) = (body(&dark.caption.normal), body(&dark.caption.inactive));
        assert!(l(off) <= l(on) + 0.01, "an inactive caption sinks");
        assert!(chroma(off) < chroma(on) / 2.0, "an inactive caption greys");
        assert!(
            light.inks.title.contrast(on) >= 4.5,
            "the title reads on the caption"
        );
        let (ls, ds) = (&light.system, &dark.system);
        for (what, a, b) in [
            ("hilight", ls.hilight, ds.hilight),
            ("hilight_text", ls.hilight_text, ds.hilight_text),
            ("menu_hilight", ls.menu_hilight, ds.menu_hilight),
            ("background", ls.background, ds.background),
        ] {
            assert_eq!(a, b, "{what} is kept");
        }
        assert_eq!(dark.inks.title, light.inks.title);
        assert_eq!(dark.inks.start_label, light.inks.start_label);
        assert_eq!(dark.inks.task_normal, light.inks.task_normal);
        assert_eq!(dark.metrics, light.metrics);
    }

    #[test]
    fn a_disabled_face_is_flat_near_the_dialog_under_a_dim_rim() {
        let d = navy();
        let face = l(d.system.button_face);
        let b = &d.push_button;
        for c in b
            .disabled
            .down
            .iter()
            .map(|s| s.1)
            .filter(|&c| c != b.disabled.rim)
        {
            assert!((l(c) - face).abs() < 0.05, "{c:?} is flat near the dialog");
        }
        let rim = l(b.disabled.rim);
        assert!(rim > face + 0.08, "the disabled rim still shows");
        assert!(rim < l(b.normal.rim) - 0.1, "the disabled rim is dim");
        let k = &d.check_box;
        let (on, off) = (
            k.normal_checked.mark.unwrap(),
            k.disabled_checked.mark.unwrap(),
        );
        assert!(
            l(off) > l(k.disabled_checked.inner) + 0.15,
            "a greyed tick shows"
        );
        assert!(chroma(off) < chroma(on), "a greyed tick loses its colour");
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
    fn a_held_arrow_lights_above_its_raised_face_in_a_dark_blue() {
        let (light, d) = (&NAVY, navy());
        for (lf, df) in [
            (&light.scroll_arrow_up, &d.scroll_arrow_up),
            (&light.combo_drop, &d.combo_drop),
            (&light.spin_up, &d.spin_up),
        ] {
            let mid = |f: &Face| f.down[f.down.len() / 2].1;
            let (was, held, rest) = (mid(&lf.pressed), mid(&df.pressed), mid(&df.normal));
            assert!(l(held) <= HELD.1 + 0.005, "{held:?} is dark, was {was:?}");
            assert!(
                l(held) > l(rest) - 0.08,
                "{held:?} stands near its raised face"
            );
            assert!(chroma(held) > 0.05, "{held:?} stays blue");
        }
    }

    #[test]
    fn the_places_bar_sinks_like_a_list_and_the_start_menu_goes_night() {
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
        for ((_, a), (_, b)) in lm.band.iter().zip(dm.band) {
            assert!(l(*b) < l(*a), "the band goes deeper");
            assert!(
                (angle(*a) - angle(*b)).abs() < 0.05,
                "the band keeps its hue"
            );
        }
        assert_eq!(dm.band_ink, lm.band_ink);
        assert!(
            l(dm.places) > l(d.system.window),
            "the places column stands above the programs"
        );
        assert!(dm.places_ink.contrast(dm.places) >= 4.5, "a place reads");
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
