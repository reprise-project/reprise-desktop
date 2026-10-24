use super::*;
use crate::raster::Canvas;
use crate::text::Align;
use reprisekit::classic::widgets::draw_label;
use reprisekit::controls::combo;
use reprisekit::cues::Cues;
use reprisekit::frame::FrameState;
use reprisekit::look::Paint;
use reprisekit::look::widgets::draw_button;
use reprisekit::pointer::{Interaction, Lit, lit_of};

/// The Run box's sunken field, drop button and value. Its caret is the
/// only one a dialog blinks, so a blink repaints just this over the kept
/// canvas; the field fills its own background, so nothing blends twice.
pub fn draw_field(
    c: &mut Canvas,
    theme: &Theme,
    text: &TextRenderer,
    layout: &DialogLayout,
    d: &Dialog,
    caret: bool,
) {
    let px = theme.fonts.size_px;
    if let (Some(fr), Some(field)) = (layout.field, &d.field) {
        let focused = d.button_focus.is_none();
        let lay = combo::layout(&field.combo, fr, &layout.controls);
        let drop = d.field_lit.of(FieldPart::Drop);
        let lit = combo::ComboLit {
            frame: match drop {
                Interaction::Idle => d.field_lit.of(FieldPart::Edit),
                lit => lit,
            },
            drop,
        };
        combo::draw(
            c,
            text,
            &theme.paint(),
            &field.combo,
            &lay,
            focused,
            false,
            reprisekit::textedit::Caret::new(focused, caret),
            lit,
            px,
        );
    }
}

/// Render `d` at `theme.scale`; `layout` is the logical layout, `icon` the
/// dialog's 32 px (times scale) icon, `caption_icon` the same icon at the
/// theme's icon size for a styled caption, `button_icons` those of a
/// Welcome panel's buttons (parallel to `d.buttons`), `row_icons` the
/// chooser's row icons or a Properties sheet's Opens with icon (the
/// first), and `logo` the Welcome panel's logo.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    theme: &Theme,
    text: &TextRenderer,
    layout: &DialogLayout,
    icon: Option<&Canvas>,
    caption_icon: Option<&Canvas>,
    button_icons: &[Option<Canvas>],
    row_icons: &[Option<&Canvas>],
    logo: Option<&Canvas>,
    d: &Dialog,
    caret: bool,
    lit: Lit<usize>,
    cues: Cues,
    // Whether the dialog has the keyboard. An inactive one draws a grey
    // caption, no default ring and no focus rectangle.
    active: bool,
    preedit: Option<&reprisekit::textedit::Preedit>,
) -> Canvas {
    // Under a look with glass the panel has a frame and draws as a dialog.
    if let (Style::Welcome, None) = (&d.style, &layout.frame) {
        return draw_welcome(theme, text, layout, button_icons, logo, d, lit, cues);
    }
    let s = theme.scale.max(1);
    let cs = &theme.colors;
    let px = theme.fonts.size_px;
    let dots = cues.focus && active;
    let paint = Paint {
        active,
        access_keys: cues.keys,
        ..theme.paint()
    };
    let (w, h) = (layout.size.0 * s, layout.size.1 * s);
    let mut c = Canvas::new(w.max(1) as u32, h.max(1) as u32);
    c.set_preedit(preedit.cloned());
    if let Some(g) = layout.frame {
        let state = FrameState {
            title: &d.title,
            active,
            maximized: false,
            pressed: d
                .caption_box
                .lit()
                .and_then(|(b, how)| (how == Interaction::Pressed).then_some(b)),
            glow: reprisekit::frame::Glow::lit(d.caption_hot),
            icon: caption_icon,
            tabbed: false,
        };
        reprisekit::look::draw_dialog_frame(
            &mut c,
            theme,
            text,
            &g.scaled(s),
            &state,
            layout.help.map(|_| d.help_box.of(())),
        );
    }

    if let (Some(p), Some(client)) = (d.picker(), layout.page) {
        p.dlg.draw(&mut c, text, &paint, client, px);
    }
    if let Some(f) = d.form() {
        draw_form(&mut c, &paint, theme, text, f, dots, row_icons);
    }
    if let (Some(ch), Some(client)) = (d.chooser(), layout.page) {
        let lay = ch
            .ow
            .layout(text, px, (client.x, client.y), &layout.controls);
        ch.ow
            .draw(&mut c, text, &paint, &lay, icon, row_icons, cues, px);
    }
    if let (Some(t), Some(client)) = (d.tray(), layout.page) {
        let lay = t.layout(text, px, (client.x, client.y));
        t.draw(&mut c, text, &paint, &lay, cues, px);
    }
    if let (Some(r), Some(ic)) = (layout.icon, icon) {
        let r = r.scaled(s);
        c.blit_over(
            r.x + (r.width - ic.width as i32) / 2,
            r.y + (r.height - ic.height as i32) / 2,
            ic,
        );
    }
    if let (
        Some(r),
        Style::Classic {
            delete_badge: true, ..
        },
    ) = (layout.icon, &d.style)
    {
        let r = r.scaled(s);
        draw_delete_badge(&mut c, r.x, r.y, s);
    }
    if let (Some(r), Some(FormKind::Properties(b))) = (layout.icon, d.form().map(|f| &f.kind))
        && b.shortcut
    {
        let r = r.scaled(s);
        reprisekit::controls::iconview::draw_shortcut_arrow(&mut c, r.x, r.bottom(), s);
    }
    // The dialog's own text and buttons, in the font it is set in; the
    // frame and a form's pages above pick their own.
    let text = text.dialog(d.font());
    for (r, line) in &layout.lines {
        text.draw(
            &mut c,
            r.scaled(s),
            line,
            false,
            cs.button_text,
            Align::Left,
            px,
        );
    }
    if let (Some(lr), Some(field)) = (layout.field_label, &d.field) {
        draw_label(
            &mut c,
            text,
            lr.scaled(s),
            &paint.label(&field.label),
            false,
            cs.button_text,
            Align::Left,
            px,
            s,
        );
    }
    draw_field(&mut c, theme, text, layout, d, caret);
    for (i, (r, b)) in layout.buttons.iter().zip(&d.buttons).enumerate() {
        // With no field to hold the focus, the default button has it and
        // wears the dotted focus rectangle. With one, a button Tab moved
        // to has it and takes the ring from the default.
        let (ring, focus) = match (&d.field, d.button_focus) {
            (None, _) => (b.default, b.default && dots),
            (Some(_), Some(f)) => (f == i, f == i && dots),
            (Some(_), None) => (b.default, false),
        };
        if b.icon.is_some() {
            let icon = button_icons.get(i).and_then(|o| o.as_ref());
            let look = IconButton {
                default: ring && active,
                ptr: lit_of(lit, i),
                focus,
            };
            draw_icon_button(
                &mut c,
                theme,
                text,
                &paint,
                r.scaled(s),
                b,
                icon,
                look,
                cues,
            );
            continue;
        }
        draw_button(
            &mut c,
            text,
            &paint,
            r.scaled(s),
            &b.label,
            ring && active,
            lit_of(lit, i),
            b.enabled,
            focus,
            px,
        );
    }
    if let (Some(f), Some(l)) = (
        d.field.as_ref().filter(|f| f.combo.open.is_some()),
        layout.list,
    ) {
        draw_list(&mut c, &paint, text, f, l, layout.scrollbar(d), px);
    }
    c
}

/// The copy progress control of `c` into `client` of a dialog drawn at `scale`, from the unscaled
/// `theme`: the control draws on a canvas of the output's scale, underlining while `keys`.
#[allow(clippy::too_many_arguments)]
pub fn draw_copy(
    canvas: &mut Canvas,
    theme: &Theme,
    text: &TextRenderer,
    c: &CopyDialog,
    client: Rect,
    scale: i32,
    active: bool,
    keys: bool,
) {
    let mut page = Canvas::with_scale(client.width as u32, client.height as u32, scale);
    let paint = Paint {
        active,
        access_keys: keys,
        s: 1,
        ..theme.paint()
    };
    c.cp.draw(&mut page, text, &paint, theme.fonts.size_px);
    canvas.blit_canvas(client.x * scale, client.y * scale, &page.at_scale(1));
}

/// The copy progress control's nodes under `ROOT`, moved from its own origin to `client`'s.
pub fn access_copy(
    c: &CopyDialog,
    client: Rect,
    theme: &Theme,
    text: &TextRenderer,
    tree: &mut reprisekit::access::Tree,
) {
    use reprisekit::access::ROOT;
    let before: std::collections::HashSet<_> = tree.nodes().map(|(id, _)| id).collect();
    c.cp.access(tree, text, &theme.paint(), theme.fonts.size_px);
    let added: Vec<_> = tree
        .nodes()
        .map(|(id, _)| id)
        .filter(|id| *id != ROOT && !before.contains(id))
        .collect();
    let (dx, dy) = (f64::from(client.x), f64::from(client.y));
    for id in added {
        if let Some(n) = tree.get_mut(id)
            && let Some(b) = n.bounds()
        {
            n.set_bounds(reprisekit::access::accesskit::Rect::new(
                b.x0 + dx,
                b.y0 + dy,
                b.x1 + dx,
                b.y1 + dy,
            ));
        }
    }
}

/// `d`'s `access` part under the dialog node `ROOT`, in dialog
/// coordinates: its text, Run's labelled field, a form's tabs and open
/// page, and the buttons. Returns the node with the focus: the field, the
/// form's focused control, else the default button. The Browse dialog
/// and the Open With chooser add their own control's tree and return its
/// focus instead.
pub fn access(
    d: &Dialog,
    layout: &DialogLayout,
    text: &TextRenderer,
    px: u32,
    tree: &mut reprisekit::access::Tree,
) -> reprisekit::access::accesskit::NodeId {
    use reprisekit::access::{self as ax, ROOT};
    if let (Some(p), Some(client)) = (d.picker(), layout.page) {
        p.dlg.access(tree, client, text, px);
        return tree.focus();
    }
    if let (Some(ch), Some(client)) = (d.chooser(), layout.page) {
        let lay = ch
            .ow
            .layout(text, px, (client.x, client.y), &layout.controls);
        return ch.ow.access(tree, &lay);
    }
    if let (Some(t), Some(client)) = (d.tray(), layout.page) {
        let lay = t.layout(text, px, (client.x, client.y));
        return t.access(tree, &lay, text, px);
    }
    let mut focus = None;
    if let Some(f) = d.form() {
        focus = match &f.body {
            FormBody::Sheet(s) => s.access(tree, ROOT, text, px).1,
            FormBody::Plain(form) => form.access(tree, ROOT, text, px),
        };
    }
    if let Some((first, _)) = layout.lines.first() {
        let all = layout.lines.iter().fold(*first, |u, (r, _)| u.union(r));
        tree.push(
            ROOT,
            "text",
            ax::label(all, &d.text.replace('\n', " "), true),
        );
    }
    if let (Some(fr), Some(field)) = (layout.field, &d.field) {
        let label = layout
            .field_label
            .map(|lr| tree.push(ROOT, "field label", ax::label(lr, &field.label, true)));
        let lay = combo::layout(&field.combo, fr, &layout.controls);
        focus = Some(combo::access(
            &field.combo,
            &lay,
            text,
            px,
            tree,
            ROOT,
            "field",
        ));
        let combo_id = reprisekit::access::Tree::id_of(ROOT, "field");
        if let (Some(l), Some(n)) = (label, tree.get_mut(combo_id)) {
            n.push_labelled_by(l);
        }
    }
    let mut default = None;
    for (i, (r, b)) in layout.buttons.iter().zip(&d.buttons).enumerate() {
        let id = tree.push(
            ROOT,
            ("button", i),
            ax::button(*r, &b.label, b.enabled, b.default),
        );
        if b.default {
            default = Some(id);
        }
        if d.button_focus == Some(i) {
            focus = Some(id);
        }
    }
    focus.or(default).unwrap_or(ROOT)
}

/// The icons a Properties sheet's open page shows in its pictures, by
/// item id and icon name, which the host resolves at 32 px and hands to
/// `draw` as row icons from 1 on (row 0 is the Opens with program).
pub fn page_pictures(d: &Dialog) -> Vec<(&'static str, String, i32)> {
    use reprisekit::controls::filesheet::{
        CUSTOM_ICON, CUSTOM_PREVIEW, PAGE_ICONS, PREVIEW_PX, STANDARD_FOLDER_ICON,
    };
    let Some(f) = d.form() else {
        return Vec::new();
    };
    let folder = match &f.kind {
        FormKind::Properties(b) => b.icon_chosen.clone().unwrap_or_else(|| b.own_icon.clone()),
        FormKind::SetProperties(s) => s.icon_chosen.clone().flatten(),
        _ => return Vec::new(),
    }
    .unwrap_or_else(|| STANDARD_FOLDER_ICON.to_string());
    let Some(page) = f.open_page() else {
        return Vec::new();
    };
    PAGE_ICONS
        .into_iter()
        .map(|(id, n)| (id, n.to_string(), ICON_PX))
        .chain([
            (CUSTOM_ICON, folder.clone(), ICON_PX),
            (CUSTOM_PREVIEW, folder, PREVIEW_PX),
        ])
        .filter(|(id, _, _)| page.item(id).is_some())
        .collect()
}

/// A form dialog: a sheet through `propsheet::draw`, or the plain form.
/// A Properties sheet's Opens with icon (row 0 of `row_icons`, from the
/// host's cache) and its page's other pictures (`page_pictures`, from
/// row 1 on) are filled in here, since the pages are built without them.
/// `p` is the dialog's paint.
fn draw_form(
    c: &mut Canvas,
    p: &Paint,
    theme: &Theme,
    text: &TextRenderer,
    f: &FormDialog,
    cues: bool,
    row_icons: &[Option<&Canvas>],
) {
    let opens_icon = row_icons.first().copied().flatten();
    let s = theme.scale.max(1);
    let px = theme.fonts.size_px;
    match &f.body {
        FormBody::Sheet(sheet) => {
            reprisekit::controls::propsheet::draw(c, text, p, sheet, cues, px)
        }
        FormBody::Plain(form) => reprisekit::controls::form::draw(c, text, p, form, cues, px),
    }
    let Some(form) = f.open_page() else {
        return;
    };
    if let (FormKind::Properties(_), Some(r), Some(ic)) =
        (&f.kind, form.rect("opens_icon"), opens_icon)
    {
        c.blit_over(r.x * s, r.y * s, ic);
    }
    if matches!(f.kind, FormKind::Properties(_) | FormKind::SetProperties(_)) {
        // In `page_pictures` order, which the host resolved from row 1 on.
        let pictures = [
            reprisekit::controls::filesheet::PAGE_ICONS[0].0,
            reprisekit::controls::filesheet::PAGE_ICONS[1].0,
            reprisekit::controls::filesheet::CUSTOM_ICON,
            reprisekit::controls::filesheet::CUSTOM_PREVIEW,
        ];
        let rects = pictures.into_iter().filter_map(|id| form.rect(id));
        for (r, ic) in rects.zip(row_icons.iter().skip(1)) {
            if let Some(ic) = ic {
                let (w, h) = (ic.width as i32, ic.height as i32);
                let r = r.scaled(s);
                c.blit_over(
                    r.x + (r.width - w).max(0) / 2,
                    r.y + (r.height - h).max(0) / 2,
                    ic,
                );
            }
        }
    }
}

/// The red exclamation badge on the Confirm Delete icon: a 12 px disc
/// with a white exclamation mark over the bottom-left corner of the 32 px
/// icon at (`x`, `y`).
pub fn draw_delete_badge(c: &mut Canvas, x: i32, y: i32, s: i32) {
    reprisekit::art::icons::DELETE_BADGE.paint(c, x + 3 * s, y + 18 * s, s);
}

fn draw_list(
    c: &mut Canvas,
    p: &Paint,
    text: &TextRenderer,
    f: &Field,
    l: Rect,
    scrollbar: Option<Rect>,
    px: u32,
) {
    combo::draw_list(
        c,
        text,
        p,
        &f.combo.items,
        f.combo.open,
        f.combo.top,
        f.combo.rows,
        l,
        scrollbar,
        None,
        px,
    );
}
