//! Blue's common controls in each variant: push buttons, check boxes, radios, group boxes, scrollbars, fields, combo drops, spinners, tabs, progress and trackbars, in their states.

use reprisekit::classic::widgets::{Arrow, CheckState};
use reprisekit::look::widgets::{
    ComboLook, draw_button, draw_calendar_band, draw_checkbox_state, draw_checkbox_state_disabled,
    draw_combo_drop, draw_combo_frame, draw_disabled_field, draw_edit_field, draw_field_frame,
    draw_group_box, draw_hscrollbar, draw_page, draw_progress, draw_radio, draw_radio_disabled,
    draw_scrollbar, draw_size_grip, draw_spin_buttons, draw_spin_field, draw_tab, draw_trackbar,
    draw_vertical_trackbar,
};
use reprisekit::pointer::{Interaction, ScrollHit};
use reprisekit::raster::{Canvas, Rect};
use reprisekit::style::{Part, State};
use reprisekit::text::{Align, TextRenderer};
use reprisekit::theme::Theme;
use reprisekit::trackbar;

const GROUP_A_H: i32 = 110;
const GROUP_B_H: i32 = 140;
const GROUP_C_H: i32 = 140;

pub const HEIGHT: i32 = GROUP_A_H + GROUP_B_H + GROUP_C_H;

pub fn draw(c: &mut Canvas, theme: &Theme, text: &TextRenderer, r: Rect) {
    draw_group_a(c, theme, text, Rect::new(r.x, r.y, r.width, GROUP_A_H));
    draw_group_b(
        c,
        theme,
        text,
        Rect::new(r.x, r.y + GROUP_A_H, r.width, GROUP_B_H),
    );
    draw_group_c(
        c,
        theme,
        text,
        Rect::new(r.x, r.y + GROUP_A_H + GROUP_B_H, r.width, GROUP_C_H),
    );
}

fn draw_group_a(c: &mut Canvas, theme: &Theme, text: &TextRenderer, r: Rect) {
    let p = theme.paint();
    let px = theme.fonts.size_px;
    let buttons = [
        ("Cancel", false, Interaction::Idle, true, false),
        ("Hot", false, Interaction::Hot, true, false),
        ("Held", false, Interaction::Pressed, true, false),
        ("Apply", false, Interaction::Idle, false, false),
        ("OK", true, Interaction::Idle, true, false),
        ("Focus", false, Interaction::Idle, true, true),
    ];
    for (i, (label, default, ptr, enabled, focus)) in buttons.into_iter().enumerate() {
        let b = Rect::new(r.x + 8 + i as i32 * 81, r.y + 6, 75, 23);
        draw_button(c, text, &p, b, label, default, ptr, enabled, focus, px);
    }
    let style = theme.look.style().expect("a Blue theme is styled");
    let helps = [
        (State::Normal, theme.colors.active_title),
        (State::Hot, theme.colors.active_title),
        (State::Pressed, theme.colors.active_title),
        (State::Inactive, theme.colors.inactive_title),
    ];
    for (i, (state, caption)) in helps.into_iter().enumerate() {
        let at = Rect::new(r.x + 500 + i as i32 * 33, r.y + 5, 29, 25);
        c.fill_rect(at, caption);
        style.draw(
            c,
            Rect::new(at.x + 4, at.y + 2, 21, 21),
            Part::HelpButton,
            state,
            1,
        );
    }
    let ptrs = [Interaction::Idle, Interaction::Hot, Interaction::Pressed];
    let mut x = r.x + 8;
    let y = r.y + 40;
    for state in [CheckState::Off, CheckState::On, CheckState::Mixed] {
        for ptr in ptrs {
            draw_checkbox_state(c, &p, Rect::new(x, y, 13, 13), state, ptr);
            x += 17;
        }
        draw_checkbox_state_disabled(c, &p, Rect::new(x, y, 13, 13), state);
        x += 25;
    }
    for checked in [false, true] {
        for ptr in ptrs {
            draw_radio(c, &p, Rect::new(x, y, 13, 13), checked, ptr);
            x += 17;
        }
        draw_radio_disabled(c, &p, Rect::new(x, y, 13, 13), checked);
        x += 25;
    }
    let g = Rect::new(r.x + 8, r.y + 66, 240, 38);
    let label = Rect::new(g.x + 10, g.y - 5, text.measure("Direction", false, px), 13);
    draw_group_box(c, text, &p, g, label, "&Direction", px);
    draw_checkbox_state_disabled(c, &p, Rect::new(g.x + 10, g.y + 15, 13, 13), CheckState::On);
    reprisekit::look::widgets::draw_disabled_label(
        c,
        text,
        &p,
        Rect::new(g.x + 26, g.y + 15, 120, 13),
        "Match &case",
        Align::Left,
        px,
        Some(Part::CheckBox),
    );
    draw_size_grip(c, &p, Rect::new(g.right() + 16, g.bottom() - 17, 17, 17));
}

/// The open tab over its page with a closed tab and a hot one beside it;
/// full, half and greyed progress bars at the heights hosts draw; the
/// calendar's weekday band in use and greyed; a trackbar at rest, under
/// the pointer, held and greyed; an upright one at rest, under the
/// pointer and held.
fn draw_group_c(c: &mut Canvas, theme: &Theme, text: &TextRenderer, r: Rect) {
    let p = theme.paint();
    let px = theme.fonts.size_px;
    let (x, y) = (r.x + 8, r.y + 6);

    let page = Rect::new(x, y + 20, 300, 70);
    draw_page(c, &p, page);
    for (i, (label, open, hot)) in [
        ("Open", true, false),
        ("Closed", false, false),
        ("Hot", false, true),
    ]
    .into_iter()
    .enumerate()
    {
        let (dy, h) = if open { (0, 20) } else { (2, 18) };
        let tab = Rect::new(x + i as i32 * 62, y + dy, 60, h);
        draw_tab(c, text, &p, tab, label, open, hot, page, px);
    }

    let bx = x + 320;
    draw_progress(c, &p, Rect::new(bx, y, 101, 13), 1.0, true);
    draw_progress(c, &p, Rect::new(bx + 110, y, 150, 16), 0.5, true);
    draw_progress(c, &p, Rect::new(bx + 110, y + 22, 150, 17), 0.3, false);
    for (i, enabled) in [true, false].into_iter().enumerate() {
        let band = Rect::new(bx + i as i32 * 150, y + 50, 140, 18);
        let ink = draw_calendar_band(c, &p, band, enabled);
        text.draw(
            c,
            band,
            "S  M  T  W  T  F  S",
            false,
            ink,
            Align::Center,
            px,
        );
    }

    let tb = trackbar::TrackBar::new(9, 18);
    for (i, (enabled, ptr)) in [
        (true, Interaction::Idle),
        (true, Interaction::Hot),
        (true, Interaction::Pressed),
        (false, Interaction::Idle),
    ]
    .into_iter()
    .enumerate()
    {
        let lay = trackbar::layout(&tb, x + i as i32 * 110, y + 100, 90);
        draw_trackbar(c, &p, &lay, enabled, false, ptr);
    }

    let thumb_w = theme.look.controls().vtrack_thumb;
    let upright = trackbar::TrackBar::new(3, 7);
    for (i, ptr) in [Interaction::Idle, Interaction::Hot, Interaction::Pressed]
        .into_iter()
        .enumerate()
    {
        let lay =
            trackbar::layout_vertical_with(&upright, x + 460 + i as i32 * 40, y + 72, 56, thumb_w);
        draw_vertical_trackbar(c, &p, &lay, true, false, ptr);
    }
}

/// Scrollbars, fields, combo boxes and spinners in their states: four
/// vertical bars (at rest, up arrow lit, thumb lit, down arrow held), four
/// horizontal ones (at rest, left arrow lit, thumb lit, and a 16 px bar
/// with its right arrow held), an edit, a greyed edit and a list frame,
/// three combo boxes (at rest, lit, greyed) and three spinners (at rest,
/// up lit, greyed).
fn draw_group_b(c: &mut Canvas, theme: &Theme, text: &TextRenderer, r: Rect) {
    let p = &theme.paint();
    let cs = &theme.colors;
    c.fill_rect(r, cs.button_face);
    let (x0, y0) = (r.x + 10, r.y + 8);
    let lit = [
        None,
        Some((ScrollHit::Back, Interaction::Hot)),
        Some((ScrollHit::Thumb, Interaction::Hot)),
        Some((ScrollHit::Forward, Interaction::Pressed)),
    ];
    for (i, lit) in lit.into_iter().enumerate() {
        let bar = Rect::new(x0 + i as i32 * 25, y0, 17, 124);
        draw_scrollbar(c, p, bar, 20, 30, 100, lit);
    }
    for (i, lit) in lit.into_iter().enumerate() {
        let thick = if i == 3 { 16 } else { 17 };
        let lit = if i == 3 {
            Some((ScrollHit::Forward, Interaction::Pressed))
        } else {
            lit
        };
        let bar = Rect::new(x0 + 110, y0 + i as i32 * 24, 210, thick);
        draw_hscrollbar(c, p, bar, 40, 50, 100, lit);
    }
    let fx = x0 + 340;
    let edit = Rect::new(fx, y0, 120, 21);
    let inner = draw_edit_field(c, p, edit, cs.window, false, Interaction::Idle);
    let size = theme.fonts.size_px;
    text.draw(c, inner, "Edit", false, cs.window_text, Align::Left, size);
    let greyed = draw_disabled_field(c, p, Rect::new(fx, y0 + 28, 120, 21));
    text.draw(c, greyed, "Greyed", false, cs.gray_text, Align::Left, size);
    let list = Rect::new(fx, y0 + 56, 120, 68);
    c.fill_rect(list, cs.window);
    draw_field_frame(c, p, list, cs.window);
    text.draw(
        c,
        Rect::new(list.x + 4, list.y + 4, 100, 16),
        "List",
        false,
        cs.window_text,
        Align::Left,
        size,
    );
    let cx = fx + 140;
    for (i, (enabled, ptr)) in [
        (true, Interaction::Idle),
        (true, Interaction::Hot),
        (false, Interaction::Idle),
    ]
    .into_iter()
    .enumerate()
    {
        let field = Rect::new(cx, y0 + i as i32 * 28, 120, 21);
        let drop = Rect::new(field.right() - 17, field.y + 2, 15, 17);
        let fill = if enabled { cs.window } else { cs.button_face };
        c.fill_rect(field, fill);
        let look = ComboLook {
            enabled,
            focused: false,
            ptr,
            editable: true,
        };
        draw_combo_frame(c, p, field, drop, fill, look);
        draw_combo_drop(c, p, drop, enabled, ptr, true);
        let ink = if enabled {
            cs.window_text
        } else {
            cs.gray_text
        };
        let label = Rect::new(field.x + 4, field.y + 2, 80, 17);
        text.draw(c, label, "Combo", false, ink, Align::Left, size);
    }
    for (i, (enabled, lit)) in [
        (true, None),
        (true, Some((Arrow::Up, Interaction::Hot))),
        (false, None),
    ]
    .into_iter()
    .enumerate()
    {
        let field = Rect::new(cx + i as i32 * 42, y0 + 88, 38, 21);
        let up = Rect::new(field.right() - 17, field.y + 2, 15, 8);
        let down = Rect::new(up.x, up.bottom() + 1, 15, 8);
        draw_spin_field(c, p, field, up, down, enabled);
        draw_spin_buttons(c, p, up, down, enabled, lit);
    }
}
