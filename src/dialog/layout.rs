use super::*;
use crate::text::wrap;
use crate::theme::TextUnits;
use reprisekit::classic::widgets::{BUTTON_H, BarHit, LINE_H};
use reprisekit::controls::combo;
use reprisekit::controls::dlgtemplate;
use reprisekit::controls::metrics::ControlMetrics;
use reprisekit::frame::{Chrome, FrameGeometry};

/// Logical-pixel positions inside the dialog; `scaled` for drawing on a
/// HiDPI output, as is for hit testing against layout coordinates.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DialogLayout {
    pub size: (i32, i32),
    /// The frame around the body in the theme's metrics; None on a Welcome
    /// panel, which draws its own.
    pub frame: Option<FrameGeometry>,
    pub caption: Rect,
    /// The caption's close button (none on a Welcome panel): 16x14, 2 px in from the caption's top
    /// and right.
    pub close: Option<Rect>,
    /// Caption ? button, 2 px left of the close button.
    pub help: Option<Rect>,
    pub icon: Option<Rect>,
    pub lines: Vec<(Rect, String)>,
    pub field_label: Option<Rect>,
    pub field: Option<Rect>,
    /// The combo box's drop button inside the field's right end, when the
    /// field has a history.
    pub drop: Option<Rect>,
    /// The drop list under the field (drawn only while open): `LIST_ROWS`
    /// rows of the look's `combo_row` inside a 1 px border.
    pub list: Option<Rect>,
    pub buttons: Vec<Rect>,
    /// A control dialog's client area.
    pub page: Option<Rect>,
    /// The look's control sizes it was laid out with, which `hit`,
    /// `scrollbar` and `access` lay the Run list out with.
    pub controls: ControlMetrics,
    /// A Blue Welcome panel's own layout, which `buttons` repeats: its cells, then Cancel.
    pub power: Option<reprisekit::powerpanel::Layout>,
}

/// What a point on a dialog lands on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogHit {
    Button(usize),
    Close,
    Help,
    Caption,
    /// The text field itself, with the point's x from the text's origin.
    Field(i32),
    /// A row of Run's completion list.
    Completion(usize),
    ListBar(BarHit),
    Drop,
    Row(usize),
    Control(reprisekit::controls::form::Hit),
    Tray(traycustomize::Hit),
}

/// The caption button `over` is on.
pub fn caption_button(over: Option<DialogHit>) -> Option<CaptionButton> {
    match over? {
        DialogHit::Close => Some(CaptionButton::Close),
        _ => None,
    }
}

/// The frame every dialog is built for: a fixed dialog border is 3 px
/// (the 2 px raised edge and one of face), a pixel thinner than a sizable
/// window's, under an 18 px caption. A look whose frame differs moves the
/// body by `client_shift`.
pub(super) const DIALOG_FRAME: i32 = dlgtemplate::FRAME;

pub(super) const CAPTION_H: i32 = dlgtemplate::CAPTION_H;

/// Where a form dialog's client area starts: inside the frame, under the
/// caption.
pub(crate) const FORM_CLIENT: (i32, i32) = (DIALOG_FRAME, DIALOG_FRAME + CAPTION_H);

pub const ICON_PX: i32 = dlgtemplate::ICON_PX;

/// End Program: a fixed 354x243 template. The 32 px icon, then the text
/// in a column inking from `EP_TEXT_X` to 18 px short of the right edge, each
/// paragraph starting at its own `EP_PARA_Y` rather than on one line grid,
/// and End Now / Cancel (69x23, 9 apart) ending 19 px from the right.
const EP_W: i32 = 354;

const EP_H: i32 = 243;

const EP_ICON: (i32, i32) = (13, 32);

const EP_TEXT_X: i32 = 58;

const EP_TEXT_RIGHT: i32 = 18;

const EP_PARA_Y: [i32; 3] = [35, 74, 120];

const EP_BUTTONS: (i32, i32) = (188, 204);

const EP_BUTTON_W: i32 = 69;

const EP_BUTTON_PITCH: i32 = 78;

/// How far `theme`'s frame puts a fixed dialog's client area from where
/// the dialog was built: inside `DIALOG_FRAME` under a `CAPTION_H`
/// caption. (0, 0) under the default metrics; Blue's taller caption moves
/// it down.
pub(super) fn client_shift(theme: &Theme) -> (i32, i32) {
    let g = FrameGeometry::layout(&theme.metrics, 1, 1, false, Chrome::DIALOG);
    (
        g.client.x - DIALOG_FRAME,
        g.client.y - DIALOG_FRAME - CAPTION_H,
    )
}

impl DialogLayout {
    /// Lay `d` out at scale 1 with the text wrapped at `theme`'s font size,
    /// inside `theme`'s frame. A form dialog must have been fitted to it
    /// first ([`Dialog::fit`]).
    pub fn compute(theme: &Theme, text: &TextRenderer, d: &Dialog) -> DialogLayout {
        DialogLayout {
            controls: theme.look.controls_at_100().grown(theme.text),
            ..Self::compute_body(theme, text, d)
        }
    }

    fn compute_body(theme: &Theme, text: &TextRenderer, d: &Dialog) -> DialogLayout {
        let body = match &d.style {
            Style::Welcome(w) if theme.glass().is_some() && !w.always_blue => {
                Self::compute_welcome_glass(d)
            }
            Style::Welcome(w) => return Self::compute_welcome(w, theme.text),
            Style::Form(f) => return Self::compute_form(theme, d, f),
            Style::TrayCustomize(t) => {
                return Self::compute_client(t.client()).framed(theme, d.help);
            }
            Style::EndProgram => Self::compute_end_program(theme, text.dialog(d.font()), d),
            Style::Classic {
                template, text_h, ..
            } => return Self::compute_template(theme, text, d, *template, *text_h),
        };
        body.grown(theme.text).framed(theme, d.help)
    }

    /// A body laid out at 100% under `DIALOG_FRAME` and a `CAPTION_H` caption, at the text size
    /// `u`: the client and its rects grow from the client's corner, the icon keeps its size.
    fn grown(mut self, u: TextUnits) -> DialogLayout {
        let (ox, oy) = FORM_CLIENT;
        let map = |r: &mut Rect| {
            let g = u.rect(Rect::new(r.x - ox, r.y - oy, r.width, r.height));
            *r = Rect::new(g.x + ox, g.y + oy, g.width, g.height);
        };
        [&mut self.field_label, &mut self.field, &mut self.page]
            .into_iter()
            .flatten()
            .for_each(map);
        if let Some(icon) = &mut self.icon {
            let g = u.at(Rect::new(icon.x - ox, icon.y - oy, icon.width, icon.height));
            *icon = Rect::new(g.x + ox, g.y + oy, g.width, g.height);
        }
        self.lines.iter_mut().for_each(|(r, _)| map(r));
        self.buttons.iter_mut().for_each(map);
        let pad = (2 * DIALOG_FRAME, DIALOG_FRAME + FORM_CLIENT.1);
        let (w, h) = u.size((self.size.0 - pad.0, self.size.1 - pad.1));
        self.size = (w + pad.0, h + pad.1);
        self
    }

    /// A body laid out for `DIALOG_FRAME` and a `CAPTION_H` caption, put
    /// inside `theme`'s frame: the body moves by `client_shift` and the
    /// dialog grows around it.
    pub(super) fn framed(mut self, theme: &Theme, help: bool) -> DialogLayout {
        let (dx, dy) = client_shift(theme);
        let mv = |r: &mut Rect| {
            r.x += dx;
            r.y += dy;
        };
        let rects = [
            &mut self.icon,
            &mut self.field_label,
            &mut self.field,
            &mut self.drop,
            &mut self.list,
            &mut self.page,
        ];
        rects.into_iter().flatten().for_each(mv);
        self.lines.iter_mut().for_each(|(r, _)| mv(r));
        self.buttons.iter_mut().for_each(mv);
        let f = DIALOG_FRAME;
        let (w, h) = (self.size.0 - 2 * f, self.size.1 - 2 * f - CAPTION_H);
        let g = FrameGeometry::layout(&theme.metrics, w, h, false, Chrome::DIALOG);
        self.with_frame(g, help)
    }

    fn with_frame(self, g: FrameGeometry, help: bool) -> DialogLayout {
        DialogLayout {
            size: g.total,
            caption: g.caption,
            close: Some(g.buttons[2]),
            help: help.then_some(g.help_box()),
            frame: Some(g),
            ..self
        }
    }

    /// A classic dialog on `template` (see [`dlgtemplate`]).
    fn compute_template(
        theme: &Theme,
        text: &TextRenderer,
        d: &Dialog,
        template: Template,
        text_h: Option<i32>,
    ) -> DialogLayout {
        let u = theme.text;
        let lay = dlgtemplate::layout(
            template,
            text,
            theme.fonts.size_px,
            &dlgtemplate::Fields {
                text: &d.text,
                icon: d.icon.is_some(),
                field: d.field.is_some(),
                buttons: &d
                    .buttons
                    .iter()
                    .map(|b| b.label.clone())
                    .collect::<Vec<_>>(),
                text_h,
            },
            u,
        )
        .grown(u);
        let mut body = DialogLayout {
            size: lay.size,
            icon: lay.icon,
            lines: lay.lines,
            field_label: lay.field_label,
            field: lay.field,
            buttons: lay.buttons,
            ..Default::default()
        }
        .framed(theme, d.help);
        if let (Some(fl), Some(box_r)) = (&d.field, body.field) {
            let cl = combo::layout(&fl.combo, box_r, &theme.look.controls_at_100().grown(u));
            body.drop = Some(cl.drop);
            body.list = Some(cl.list);
        }
        body
    }

    /// End Program on its fixed template: each `\n`-separated paragraph
    /// of the text wraps in the column and starts at its own height.
    fn compute_end_program(theme: &Theme, text: &TextRenderer, d: &Dialog) -> DialogLayout {
        let px = theme.fonts.size_px;
        let width = EP_W - EP_TEXT_RIGHT - EP_TEXT_X;
        // The column is laid out at its ink: the second dialog font's pen
        // sits a column left of it.
        let x = EP_TEXT_X - text.lead();
        let lines = d
            .text
            .split('\n')
            .zip(EP_PARA_Y)
            .flat_map(|(para, y)| {
                wrap(text, para, theme.text.len(width), px)
                    .into_iter()
                    .enumerate()
                    .map(move |(i, s)| (Rect::new(x, y + i as i32 * LINE_H, width, LINE_H), s))
            })
            .collect();
        let buttons = (0..d.buttons.len() as i32)
            .map(|i| {
                Rect::new(
                    EP_BUTTONS.0 + i * EP_BUTTON_PITCH,
                    EP_BUTTONS.1,
                    EP_BUTTON_W,
                    BUTTON_H,
                )
            })
            .collect();
        DialogLayout {
            size: (EP_W, EP_H),
            icon: Some(Rect::new(EP_ICON.0, EP_ICON.1, ICON_PX, ICON_PX)),
            lines,
            buttons,
            ..Default::default()
        }
    }

    /// A control's client area `client` inside the fixed 3 px frame under
    /// the caption, as `page`: the tray's Customize dialog.
    pub(super) fn compute_client(client: (i32, i32)) -> DialogLayout {
        let (cw, ch) = client;
        DialogLayout {
            size: (cw + 2 * DIALOG_FRAME, ch + DIALOG_FRAME + FORM_CLIENT.1),
            page: Some(Rect::new(FORM_CLIENT.0, FORM_CLIENT.1, cw, ch)),
            ..Default::default()
        }
    }

    /// A form dialog: the caption and its fixed size; the controls carry their own rects.
    fn compute_form(theme: &Theme, d: &Dialog, f: &FormDialog) -> DialogLayout {
        let (tw, th) = f.grown_size(theme.text);
        let (w, h) =
            FrameGeometry::client_size_for_total(&theme.metrics, tw, th, false, Chrome::DIALOG);
        let g = FrameGeometry::layout(&theme.metrics, w, h, false, Chrome::DIALOG);
        // The dialog's icon (a message box's question mark, Power Options'
        // plug) sits where the form's "icon" picture is; a form without
        // one shows none.
        DialogLayout {
            icon: f.form.rect("icon"),
            ..Default::default()
        }
        .with_frame(g, d.help)
    }

    /// Run's open completion list and its place, inside the dialog under its caption.
    pub fn completion<'d>(&self, d: &'d Dialog) -> Option<(&'d Field, pathcomplete::Layout)> {
        let f = d.field.as_ref().filter(|f| f.complete.is_open())?;
        let (w, h) = self.size;
        let top = self.caption.bottom();
        let room = Rect::new(
            DIALOG_FRAME,
            top,
            w - 2 * DIALOG_FRAME,
            h - top - DIALOG_FRAME,
        );
        let lay = pathcomplete::layout(&f.complete, self.field?, Some(room), &self.controls);
        Some((f, lay))
    }

    pub fn scrollbar(&self, d: &Dialog) -> Option<Rect> {
        self.list?;
        let f = d.field.as_ref()?;
        combo::layout(&f.combo, self.field?, &self.controls).scrollbar
    }

    /// What the dialog has at (`x`, `y`). The renderer and font size go in
    /// because a hit inside a form's edit field resolves to a byte index
    /// in its value, which takes measuring the text.
    pub fn hit(
        &self,
        d: &Dialog,
        x: i32,
        y: i32,
        text: &TextRenderer,
        px: u32,
    ) -> Option<DialogHit> {
        if self.close.is_some_and(|r| r.contains(x, y)) {
            return Some(DialogHit::Close);
        }
        if self.help.is_some_and(|r| r.contains(x, y)) {
            return Some(DialogHit::Help);
        }
        // The caption, less the buttons already taken out of it above: the
        // grip a dialog is dragged by. Welcome panels are not draggable.
        if !matches!(d.style, Style::Welcome(_)) && self.caption.contains(x, y) {
            return Some(DialogHit::Caption);
        }
        if let Some(f) = d.form() {
            return f.form.hit(x, y, text, px).map(DialogHit::Control);
        }
        if let (Some(t), Some(client)) = (d.tray(), self.page) {
            let lay = t.layout(text, px, (client.x, client.y));
            return t.hit(&lay, x, y).map(DialogHit::Tray);
        }
        if let Some((f, lay)) = self.completion(d)
            && lay.list.contains(x, y)
        {
            return pathcomplete::hit(&f.complete, &lay, x, y).map(DialogHit::Completion);
        }
        if let (Some(f), Some(fr)) = (d.field.as_ref(), self.field)
            && f.combo.open.is_some()
        {
            let lay = combo::layout(&f.combo, fr, &self.controls);
            if lay.list.contains(x, y) {
                return combo::hit(&f.combo, &lay, x, y).map(|h| match h {
                    combo::Hit::Row(i) => DialogHit::Row(i),
                    combo::Hit::Field | combo::Hit::Drop => DialogHit::Drop,
                    h => h.bar().map_or(DialogHit::Drop, DialogHit::ListBar),
                });
            }
        }
        if self.drop.is_some_and(|r| r.contains(x, y)) {
            return Some(DialogHit::Drop);
        }
        if let (Some(f), Some(r)) = (d.field.as_ref(), self.field.filter(|r| r.contains(x, y))) {
            let origin = combo::layout(&f.combo, r, &self.controls).text.x;
            return Some(DialogHit::Field(x - origin));
        }
        self.buttons
            .iter()
            .position(|r| r.contains(x, y))
            .map(DialogHit::Button)
    }
}
