use crate::controls::combo::{self, ComboLit};
use crate::flyout;
use crate::look::Paint;
use crate::look::widgets::{
    draw_disabled_label, draw_focus_ring, draw_glyph_button, draw_hscrollbar, draw_label,
    draw_media_button, draw_vertical_trackbar,
};
use crate::raster::Canvas;
use crate::shape::Glyph;
use crate::text::{Align, DialogFont, TextRenderer};
use crate::textedit::Caret;
use crate::theme::Theme;

use super::layout::{ColumnLayout, Layout, StripLayout};
use super::{
    Column, Dir, Focus, INPUT, MAX_SHOWN, NAME_PAD, NowPlaying, OUTPUT, Part, SPEAKER, STEPS,
    Transport, VolumePopup, combo_focus,
};

/// The panel, top to bottom; the focus ring shows once a key was pressed, and an open device list draws last, over the columns.
pub fn draw(c: &mut Canvas, theme: &Theme, text: &TextRenderer, vp: &VolumePopup, lay: &Layout) {
    let text = text.dialog(DialogFont::ShellDlg);
    vp.styled.set(theme.look.lights_pointer());
    let p = &Paint {
        s: 1,
        ..theme.paint()
    };
    let cs = p.cs;
    let px = theme.fonts.size_px;
    // The panel is a program's own flyout: no blurred backdrop under it.
    let one;
    let at_1 = if theme.scale == 1 {
        theme
    } else {
        one = Theme {
            scale: 1,
            ..theme.clone()
        };
        &one
    };
    flyout::draw_shadow(at_1, c, lay.panel);
    flyout::draw_panel(at_1, c, lay.panel, cs.button_face, flyout::Backdrop::None);
    for (dir, label, r) in [
        (Dir::Output, OUTPUT, lay.output_label),
        (Dir::Input, INPUT, lay.input_label),
    ] {
        let combo = vp.combo(dir);
        if combo.enabled {
            draw_label(c, text, p, r, label, cs.button_text, Align::Right, px);
        } else {
            draw_disabled_label(c, text, p, r, label, Align::Right, px, None);
        }
        let lit = vp.ptr.of(Part::Combo(dir));
        combo::draw(
            c,
            text,
            p,
            combo,
            lay.combo(dir),
            vp.focus == combo_focus(dir),
            vp.cues,
            Caret::default(),
            ComboLit {
                frame: lit,
                drop: lit,
            },
            px,
        );
    }
    let sep = lay.separator;
    c.etched_divider_t(sep.x, sep.right() - 1, sep.y, cs, 1);
    if let Some(d) = lay.divider {
        c.etched_vline_t(d.x, d.y, d.bottom(), cs, 1);
    }
    for (i, (col, cl)) in vp.columns.iter().zip(&lay.columns).enumerate() {
        if let Some(cl) = cl {
            draw_column(c, text, p, vp, i, col, cl, px);
        }
    }
    if let Some(bar) = lay.bar {
        let lit = match vp.ptr.lit() {
            Some((Part::Bar(h), how)) => Some((h, how)),
            _ => None,
        };
        let apps = vp.columns.len() - 1;
        draw_hscrollbar(c, p, bar, vp.first, MAX_SHOWN, apps, lit);
    }
    if let (Some(np), Some(st)) = (&vp.playing, &lay.strip) {
        draw_strip(c, text, p, vp, np, st, px);
    }
    if let Some(d) = vp.open_list() {
        let (combo, cl) = (vp.combo(d), lay.combo(d));
        combo::draw_list_rows(
            c,
            text,
            p,
            &combo.items,
            combo.open,
            combo.top,
            combo.rows,
            cl.list,
            cl.scrollbar,
            None,
            px,
            cl.row_h,
        );
    }
}

/// A column's level in percent.
pub(super) fn level_text(col: &Column) -> String {
    format!("{}%", col.volume.pos * 100 / (STEPS - 1))
}

#[expect(clippy::too_many_arguments)]
pub(super) fn draw_column(
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    vp: &VolumePopup,
    i: usize,
    col: &Column,
    cl: &ColumnLayout,
    px: u32,
) {
    let cs = p.cs;
    let live = col.volume.enabled;
    if let (Some(icon), Some(r)) = (&col.icon, cl.icon) {
        c.blit_over(r.x, r.y, icon);
    }
    if !live {
        draw_disabled_label(c, text, p, cl.label, &col.label, Align::Center, px, None);
    } else if col.key.is_none() {
        draw_label(
            c,
            text,
            p,
            cl.label,
            &col.label,
            cs.button_text,
            Align::Center,
            px,
        );
    } else {
        // A program's name is plain text: an `&` in it is no access key.
        let name = text.ellipsize(&col.label, cl.label.width - 2 * NAME_PAD, false, px);
        text.draw(c, cl.label, &name, false, cs.button_text, Align::Center, px);
    }
    let level = level_text(col);
    if live && !col.mute {
        text.draw(
            c,
            cl.level,
            &level,
            false,
            cs.button_text,
            Align::Center,
            px,
        );
    } else {
        draw_disabled_label(c, text, p, cl.level, &level, Align::Center, px, None);
    }
    draw_vertical_trackbar(
        c,
        p,
        &cl.track,
        live && !col.mute,
        vp.focus == Focus::Thumb(i),
        vp.ptr.of(Part::Thumb(i)),
    );
    let glyph = if col.mute {
        Glyph::SpeakerMuted
    } else {
        Glyph::Speaker
    };
    draw_glyph_button(
        c,
        text,
        p,
        cl.mute,
        (glyph, SPEAKER),
        live,
        vp.ptr.of(Part::Mute(i)),
        live && vp.cues && vp.focus == Focus::Mute(i),
        px,
    );
    if vp.cues && live && vp.focus == Focus::Thumb(i) {
        draw_focus_ring(c, p, cl.track.bounds, cs.button_text);
    }
}

pub(super) fn draw_strip(
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    vp: &VolumePopup,
    np: &NowPlaying,
    st: &StripLayout,
    px: u32,
) {
    let cs = p.cs;
    let sep = st.separator;
    c.etched_divider_t(sep.x, sep.right() - 1, sep.y, cs, 1);
    let lines = [(st.title, &np.title)]
        .into_iter()
        .chain(st.artist.map(|r| (r, &np.artist)));
    for (r, line) in lines {
        let cut = text.ellipsize(line, r.width, false, px);
        text.draw(c, r, &cut, false, cs.button_text, Align::Left, px);
    }
    for t in Transport::ALL {
        let (glyph, _) = np.glyph(t);
        draw_media_button(
            c,
            text,
            p,
            st.button(t),
            glyph,
            np.can(t),
            vp.ptr.of(Part::Media(t)),
            vp.cues && vp.focus == Focus::Media(t),
            px,
        );
    }
}
