use crate::art::clockface;
use crate::classic::widgets::{
    self, Axis, CheckState, LINE_H, draw_focus_ring, draw_label, edit_first_visible,
};
use crate::controls::arrange;
use crate::controls::calendar;
use crate::controls::colorbutton;
use crate::controls::combo;
use crate::controls::hotkey;
use crate::controls::iconlist;
use crate::controls::listbox;
use crate::controls::multiedit;
use crate::controls::timefield;
use crate::controls::tooltip;
use crate::controls::treeview;
use crate::controls::updown;
use crate::look::Paint;
use crate::look::widgets::{
    draw_button, draw_checkbox_state, draw_checkbox_state_disabled, draw_disabled_field,
    draw_disabled_label, draw_edit_field, draw_edit_text, draw_group_box, draw_radio,
    draw_radio_disabled, draw_trackbar, draw_well, group_label_ink,
};
use crate::pointer::Interaction;
use crate::raster::{Canvas, EdgeStyle, Rect};
use crate::text::{Align, TextRenderer};
use crate::textedit::Caret;
use crate::theme::{Color, ColorScheme, Theme};
use crate::trackbar;

use super::field::{masked, visible_from};
use super::item::Screen;
use super::layout::{
    check_box, check_label_rect, choice_label_room, glyph_dy, icon_list_layout, note_lines,
    radio_box, radio_label_rect,
};
use super::{Control, Form, Light, Memo};

/// The scheme a greyed field's value is inked in: `None` for a live field or when the look keeps `button_text`.
pub(crate) fn greyed_field_scheme(p: &Paint, enabled: bool) -> Option<ColorScheme> {
    (!enabled)
        .then(|| p.look.style())
        .flatten()
        .and_then(|st| {
            st.text_color(
                crate::style::Part::EditBorder,
                crate::style::State::Disabled,
            )
        })
        .map(|ink| ColorScheme {
            button_text: ink,
            ..p.cs.clone()
        })
}

impl Form {
    /// A copy drawn with its (0, 0) at `origin` on the surface, the pointer rest moved along.
    pub(crate) fn placed_at(&self, origin: (i32, i32)) -> Form {
        let mut f = self.clone();
        let (dx, dy) = (origin.0 - self.origin.0, origin.1 - self.origin.1);
        f.origin = origin;
        f.point = self.point.map(|(x, y)| (x + dx, y + dy));
        f
    }

    /// Take what `drawn`, a [`Form::placed_at`] copy of this form, laid out when it drew: the
    /// labels it cut, its text px and how far it moved and widened items for their text.
    pub(crate) fn note_draw(&self, drawn: &Form) {
        let r = &drawn.rendered;
        self.rendered
            .ellipsized
            .replace(r.ellipsized.borrow().clone());
        self.rendered.fit.replace(r.fit.borrow().clone());
        self.rendered.px.set(r.px.get());
    }
}

/// A check box's or radio's one-line label at `lr`'s origin (device px,
/// as tall as the item's row), greyed as the style's `part` greys it when
/// not `enabled`, with the focus dots round it when `cue`.
#[expect(clippy::too_many_arguments)]
fn draw_choice_label(
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    lr: Rect,
    label: &str,
    enabled: bool,
    part: crate::style::Part,
    cue: bool,
    px: u32,
) {
    let (cs, s) = (p.cs, p.s);
    let (plain, _) = crate::text::split_mnemonic(label);
    let lw = text.measure(&plain, false, px);
    let r = Rect::new(lr.x, lr.y, lw + 2 * s, lr.height);
    if enabled {
        draw_label(
            c,
            text,
            r,
            &p.label(label),
            false,
            cs.button_text,
            Align::Left,
            px,
            s,
        );
    } else {
        draw_disabled_label(c, text, p, r, label, Align::Left, px, Some(part));
    }
    if cue {
        draw_focus_ring(
            c,
            Rect::new(r.x - s, r.y, r.width + 2 * s, r.height),
            s,
            cs.button_text,
        );
    }
}

/// Draw the form onto `c`, already filled with face (and the page `p.back` names on a
/// sheet). Popups draw last. `cues` shows the dotted focus rectangles, hidden until the
/// keyboard is used.
pub fn draw(c: &mut Canvas, text: &TextRenderer, p: &Paint, form: &Form, cues: bool, px: u32) {
    let Paint { cs, s, .. } = *p;
    form.drawn_with(p);
    form.rendered.px.set(px);
    form.fit_text(text, px);
    form.lay_out_notes(text, s, px);
    form.rendered.ellipsized.borrow_mut().clear();
    let cm = form.controls();
    let grow = p.text;
    let text = text.dialog(form.font);
    let choices = form
        .items
        .iter()
        .any(|it| matches!(it.control, Control::Check { .. } | Control::Radio { .. }));
    let room = choices.then(|| form.choice_room(text, px));
    let at_ink = |r: Rect| {
        if form.labels_at_ink {
            Rect::new(r.x - text.lead() * s, r.y, r.width, r.height)
        } else {
            r
        }
    };
    for (i, it) in form.items.iter().enumerate() {
        let focused = form.focus == Some(i);
        let cue = focused && cues;
        let placed = form.at(it);
        let r = placed.scaled(s);
        let clip = form.clip_of(it);
        if clip.is_some_and(|v| v.intersect(&placed).is_empty()) {
            continue;
        }
        let draw_item = |c: &mut Canvas| match &it.control {
            Control::Label {
                text: label,
                enabled,
                align,
            } => {
                let r = if *align == Align::Left { at_ink(r) } else { r };
                // The fit may widen a label past the page or window that shows it.
                let shown = clip
                    .map_or(c.bounds(), |v| v.scaled(s).intersect(&c.bounds()))
                    .right();
                let label = &*ellipsized(form, i, text, label, r.right().min(shown) - r.x, px);
                if *enabled {
                    draw_label(
                        c,
                        text,
                        r,
                        &p.label(label),
                        false,
                        cs.button_text,
                        *align,
                        px,
                        s,
                    );
                } else {
                    draw_disabled_label(c, text, p, r, label, *align, px, None);
                }
            }
            Control::Lines(lines) | Control::GreyedLines(lines) => {
                let r = at_ink(r);
                let lines = note_lines(text, lines, r.width, px);
                let live = matches!(it.control, Control::Lines(_));
                for (k, line) in lines.iter().enumerate() {
                    let (top, bottom) = (
                        grow.len(k as i32 * LINE_H),
                        grow.len((k as i32 + 1) * LINE_H),
                    );
                    let lr = Rect::new(r.x, r.y + top * s, r.width, (bottom - top) * s);
                    if live {
                        text.draw(c, lr, line, false, cs.button_text, Align::Left, px);
                    } else {
                        // The greyed painter reads access keys; a note has none.
                        let line = crate::text::escape_mnemonic(line);
                        draw_disabled_label(c, text, p, lr, &line, Align::Left, px, None);
                    }
                }
            }
            Control::GroupTitle(title) => {
                let dy = p.look.style().map_or(0, |st| st.group_label.nudge.1);
                let r = at_ink(Rect::new(r.x, r.y + dy * s, r.width, r.height));
                let ink = group_label_ink(p);
                draw_label(c, text, r, &p.label(title), false, ink, Align::Left, px, s);
            }
            Control::Group(label) => {
                // A group box's label is laid out at its ink, 10 px in:
                // the UI font's pen sits there, the second dialog font's
                // a column left. The line's break opens 2 px before the
                // pen.
                let lw = text.measure(&crate::text::split_mnemonic(label).0, false, px);
                let lx = r.x + (grow.len(10) - text.lead()) * s;
                let lr = Rect::new(lx, r.y - grow.len(5) * s, lw, grow.len(LINE_H) * s);
                draw_group_box(c, text, p, r, lr, label, px);
            }
            Control::Separator => c.etched_divider_t(r.x, r.right() - s, r.y, cs, s),
            Control::Rule => c.fill_rect(r, cs.button_shadow),
            Control::Blank => p.fill_back(c, r),
            Control::Check {
                label,
                checked,
                enabled,
                mixed,
            } => {
                let b = check_box(placed, &cm, glyph_dy(it, placed)).scaled(s);
                let state = match (*mixed, *checked) {
                    (Some(true), _) => CheckState::Mixed,
                    (_, true) => CheckState::On,
                    (_, false) => CheckState::Off,
                };
                if *enabled {
                    let ptr = form.pointer.of((i, Light::Body));
                    draw_checkbox_state(c, p, b, state, ptr);
                } else {
                    draw_checkbox_state_disabled(c, p, b, state);
                }
                let lr = check_label_rect(placed, it.indent, &cm, 0);
                let room = choice_label_room(room.as_ref(), i, lr.x, clip, c, s);
                let label = &*ellipsized(form, i, text, label, room, px);
                let part = crate::style::Part::CheckBox;
                draw_choice_label(c, text, p, lr.scaled(s), label, *enabled, part, cue, px);
            }
            Control::Radio {
                label,
                checked,
                enabled,
                ..
            } => {
                let b = radio_box(placed, &cm, glyph_dy(it, placed)).scaled(s);
                if *enabled {
                    let ptr = form.pointer.of((i, Light::Body));
                    draw_radio(c, p, b, *checked, ptr);
                } else {
                    draw_radio_disabled(c, p, b, *checked);
                }
                let lr = radio_label_rect(placed, &cm, 0);
                let room = choice_label_room(room.as_ref(), i, lr.x, clip, c, s);
                let label = &*ellipsized(form, i, text, label, room, px);
                let part = crate::style::Part::Radio;
                draw_choice_label(c, text, p, lr.scaled(s), label, *enabled, part, cue, px);
            }
            Control::Edit {
                edit,
                first,
                enabled,
            } => {
                let inner = if *enabled {
                    let ptr = form.pointer.of((i, Light::Body));
                    let inner = draw_edit_field(c, p, r, cs.window, focused, ptr);
                    c.fill_rect(inner, cs.window);
                    inner
                } else {
                    draw_disabled_field(c, p, r)
                };
                // A field without the focus hides its selection until the
                // focus comes back.
                let hidden;
                let unselected;
                let shown = if edit.is_secret() {
                    hidden = masked(edit);
                    &hidden
                } else if !focused && edit.has_selection() {
                    let mut e = edit.clone();
                    let caret = e.caret();
                    e.set_selection(caret, caret);
                    unselected = e;
                    &unselected
                } else {
                    edit
                };
                // Kept in step here because only this place holds what computes it; `inner` is the text
                // area in device px.
                // A drag resting on the field scrolls it (`drag_tick`), so the caret no longer pins
                // the view.
                let drop = form.field_drop.filter(|d| d.item == i);
                let from = visible_from(shown, first);
                let from = match drop {
                    Some(_) => from,
                    None => edit_first_visible(text, shown, inner.width, px, from),
                };
                first.set(from);
                let greyed = greyed_field_scheme(p, *enabled);
                let ep = match &greyed {
                    Some(gcs) => Paint { cs: gcs, ..*p },
                    None => *p,
                };
                draw_edit_text(
                    c,
                    text,
                    shown,
                    inner,
                    px,
                    &ep,
                    Caret::new(focused && *enabled, form.caret && drop.is_none()),
                    from,
                );
                if let Some(d) = drop {
                    let tail = &shown.text[from..];
                    let at = from + crate::textedit::index_at(text, tail, d.x * s - inner.x, px);
                    widgets::draw_edit_drop_point(c, text, shown, inner, px, p, from, at);
                }
            }
            Control::Combo(combo) => {
                let lay = combo::layout(combo, placed, &cm);
                let drop = form.pointer.of((i, Light::Drop));
                let frame = match drop {
                    Interaction::Idle => form.pointer.of((i, Light::Field)),
                    lit => lit,
                };
                combo::draw(
                    c,
                    text,
                    p,
                    combo,
                    &lay,
                    focused,
                    cue,
                    Caret::Unfocused,
                    combo::ComboLit { frame, drop },
                    px,
                );
            }
            Control::ListBox { list, icons } => {
                let lay = listbox::layout(list, placed, &cm);
                listbox::draw(c, text, p, list, &lay, icons, form.bar_lit(i), px);
            }
            Control::Tree(tv) => {
                let lay = treeview::layout_without_text(tv, placed, &cm);
                let bars = form
                    .bar_lit(i)
                    .map(|(hit, how)| ((Axis::Vertical, hit), how));
                treeview::draw(c, text, p, tv, &lay, bars, px);
            }
            Control::UpDown(u) => {
                let lay = updown::layout_for(u, placed, &cm, grow);
                updown::draw(c, text, p, u, &lay, focused, form.spin_lit(i), px);
            }
            Control::TrackBar(t) => {
                let at = placed;
                let lay = trackbar::layout(t, at.x, at.y, at.width);
                draw_trackbar(
                    c,
                    p,
                    &lay,
                    t.enabled,
                    focused,
                    form.pointer.of((i, Light::Thumb)),
                );
                if cue {
                    draw_focus_ring(
                        c,
                        Rect::new(r.x - 4 * s, r.y - 2 * s, r.width + 8 * s, r.height + 4 * s),
                        s,
                        cs.button_text,
                    );
                }
            }
            Control::Button {
                label,
                enabled,
                default,
            } => {
                let ring = if form
                    .focus
                    .is_some_and(|f| matches!(form.items[f].control, Control::Button { .. }))
                {
                    focused
                } else {
                    *default
                };
                draw_button(
                    c,
                    text,
                    p,
                    r,
                    label,
                    ring,
                    form.pointer.of((i, Light::Body)),
                    *enabled,
                    cue,
                    px,
                );
            }
            Control::Picture { canvas, frame } => {
                let at = (r.x + frame * s, r.y + frame * s);
                match frame {
                    2 => draw_well(c, p, r, EdgeStyle::ButtonSunken),
                    1 => draw_well(c, p, r, EdgeStyle::ThinSunken),
                    _ => {}
                }
                if let Some(pic) = canvas {
                    c.blit_over(at.0, at.1, pic);
                }
            }
            Control::Spectrum => {
                draw_well(c, p, r, EdgeStyle::ThinSunken);
                let bar =
                    form.rendered
                        .get(i, s, (it.rect.width, it.rect.height), &it.control, || {
                            vec![crate::classic::assets::spectrum(s)]
                        });
                c.blit_canvas(r.x + s, r.y + s, &bar[0]);
            }
            Control::Monitor(screen) => {
                let (sx, sy, sw, sh) = crate::classic::assets::MONITOR_SCREEN;
                let area = Rect::new(r.x + sx * s, r.y + sy * s, sw * s, sh * s);
                let pics =
                    form.rendered
                        .get(i, s, (it.rect.width, it.rect.height), &it.control, || {
                            let fitted = match screen {
                                Screen::Picture(pic) => {
                                    pic.resample_box(area.width as u32, area.height as u32)
                                }
                                _ => Canvas::new(0, 0),
                            };
                            let bezel = crate::classic::assets::monitor();
                            let bezel =
                                bezel.resample_box(bezel.width * s as u32, bezel.height * s as u32);
                            vec![fitted, bezel]
                        });
                match screen {
                    Screen::Black => c.fill_rect(area, Color::rgb(0, 0, 0)),
                    Screen::Color(col) => c.fill_rect(area, *col),
                    Screen::Picture(_) => c.blit_canvas(area.x, area.y, &pics[0]),
                }
                c.blit_over(r.x, r.y, &pics[1]);
            }
            Control::Sample {
                themes,
                theme,
                icon,
            } => {
                let size = (it.rect.width - 4, it.rect.height - 4);
                let pic = form.rendered.get(i, s, size, &it.control, || {
                    vec![crate::look::draw_sample_at(
                        *themes,
                        theme,
                        s,
                        text,
                        icon.as_ref(),
                        size,
                    )]
                });
                draw_well(c, p, r, EdgeStyle::ButtonSunken);
                c.blit_canvas(r.x + 2 * s, r.y + 2 * s, &pic[0]);
            }
            Control::Color(b) => colorbutton::draw(c, p, placed, b),
            Control::Calendar(cal) => {
                let lay = calendar::layout(placed, &cm);
                calendar::draw(c, text, p, cal, &lay, cue, px);
            }
            Control::Clock(face) => clockface::draw(c, p, placed, *face),
            Control::Time(tf) => {
                let lay = timefield::layout(placed, &cm);
                timefield::draw(c, text, p, tf, &lay, focused, form.spin_lit(i), px);
            }
            Control::HotKey(h) => {
                let ptr = form.pointer.of((i, Light::Body));
                let field = hotkey::layout(placed);
                hotkey::draw(c, text, p, h, field, focused, form.caret, ptr, px);
            }
            Control::IconList(l) => {
                let lay = icon_list_layout(it.grows, placed, &cm);
                iconlist::draw(c, p, l, &lay, cue, cm.thumb_min);
            }
            Control::Arrange(a) => {
                let lay = arrange::layout(placed, a);
                arrange::draw(c, text, p, a, &lay, cue, px);
            }
            Control::Memo(memo) => {
                let m = Memo::metrics(px, &cm, grow);
                let lay = multiedit::layout(text, &m, &memo.edit, r, s);
                memo.rows.set(lay.rows);
                let mut view = memo.view.get();
                let caret = memo.edit.caret();
                if memo.followed.get() != Some(caret) {
                    view.follow(text, &m, &memo.edit, &lay, s);
                    memo.view.set(view);
                    memo.followed.set(Some(caret));
                }
                let greyed = (!memo.enabled).then(|| {
                    let base = greyed_field_scheme(p, false).unwrap_or_else(|| cs.clone());
                    ColorScheme {
                        window: cs.button_face,
                        ..base
                    }
                });
                let mp = match &greyed {
                    Some(gcs) => Paint { cs: gcs, ..*p },
                    None => *p,
                };
                let caret = Caret::new(focused && memo.enabled, form.caret);
                multiedit::draw(c, text, &mp, &m, &memo.edit, view, &lay, caret);
            }
        };
        match clip {
            Some(v) => c.with_clip(v.scaled(s), draw_item),
            None => draw_item(c),
        }
    }
    draw_open_popups(c, text, p, form, px);
    if let (Some(text), Some(at)) = (form.label_tip(), form.point) {
        c.report_tip(crate::raster::ReportedTip { text, at });
    }
}

/// `label`, or when its ink outruns `width` its text cut to end in an ellipsis (keeping its
/// access key when the cut leaves it), noted for [`Form::label_tip`].
pub(super) fn ellipsized<'a>(
    form: &Form,
    i: usize,
    text: &TextRenderer,
    label: &'a str,
    width: i32,
    px: u32,
) -> std::borrow::Cow<'a, str> {
    let (plain, key) = crate::text::split_mnemonic(label);
    if text.ink_width(&plain, false, px) <= width {
        return label.into();
    }
    let mut noted = form.rendered.ellipsized.borrow_mut();
    if !noted.contains(&i) {
        noted.push(i);
    }
    let cut = text.ellipsize(&plain, width, false, px);
    let kept = cut.chars().count().saturating_sub(3);
    let mut out = String::with_capacity(cut.len() + 2);
    for (k, ch) in cut.chars().enumerate() {
        if key == Some(k) && k < kept {
            out.push('&');
        }
        if ch == '&' {
            out.push('&');
        }
        out.push(ch);
    }
    out.into()
}

/// The tip a form drawn into `c` reported ([`Canvas::report_tip`]), kept inside `c` and wrapped
/// to its width; the rect it covers. `theme` is the one the form draws in. The program runtime
/// and the compositor's dialogs call it after the window's draw, so it lands over everything.
pub fn draw_tip(c: &mut Canvas, text: &TextRenderer, theme: &Theme) -> Option<Rect> {
    let crate::raster::ReportedTip {
        text: full,
        at: (x, y),
    } = c.take_tip()?;
    let s = theme.scale.max(1);
    let bounds = c.bounds();
    let frame = tooltip::tip_size(text, theme, "").0;
    let lines = crate::text::wrap(text, &full, bounds.width - 2 - frame, theme.fonts.size_px);
    let label = lines.join("\n");
    let size = tooltip::tip_size(text, theme, &label);
    let at = tooltip::place((x * s, y * s), size, bounds, true);
    c.blit_over(
        at.0,
        at.1,
        &tooltip::draw_tip_at(theme, text, &label, c.scale()),
    );
    Some(Rect::new(at.0, at.1, size.0, size.1))
}

/// The form's open popups, which [`draw`] puts over the other controls. A host drawing its
/// own control over the form draws them again after it.
pub fn draw_popups(c: &mut Canvas, text: &TextRenderer, p: &Paint, form: &Form, px: u32) {
    draw_open_popups(c, text.dialog(form.font), p, form, px);
}

fn draw_open_popups(c: &mut Canvas, text: &TextRenderer, p: &Paint, form: &Form, px: u32) {
    form.drawn_with(p);
    let cm = form.controls();
    for (i, it) in form.items.iter().enumerate() {
        match &it.control {
            Control::Combo(combo) if combo.open.is_some() && !form.hosted => {
                let lay = combo::layout(combo, form.at(it), &cm);
                combo::draw_list(
                    c,
                    text,
                    p,
                    &combo.items,
                    combo.open,
                    combo.top,
                    combo.rows,
                    lay.list,
                    lay.scrollbar,
                    form.bar_lit(i),
                    px,
                );
            }
            Control::Color(b) if b.open && !form.hosted => {
                colorbutton::draw_popup(c, text, p, form.at(it), b, px)
            }
            _ => {}
        }
    }
}
