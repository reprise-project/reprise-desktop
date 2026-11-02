//! Attributes, Resize and Skew, Flip and Rotate, the text toolbar, About and the prompts.

use super::*;
use reprisekit::classic::widgets::{BUTTON_H, BUTTON_W};
use reprisekit::controls::combo::Combo;
use reprisekit::controls::form::{Control, Item};
use reprisekit::controls::msgbox;

/// The largest side a picture may be given, in pixels.
pub const MAX_SIDE: i32 = 16384;

const OK: &str = "ok";
const CANCEL: &str = "cancel";
const WIDTH: &str = "width";
const HEIGHT: &str = "height";
const RESIZE_H: &str = "resize-h";
const RESIZE_V: &str = "resize-v";
const SKEW_H: &str = "skew-h";
const SKEW_V: &str = "skew-v";
const FLIPS: [&str; 5] = ["flip-h", "flip-v", "rot-90", "rot-180", "rot-270"];
const FAMILY: &str = "family";
const SIZE: &str = "size";
const BOLD: &str = "bold";

pub const FONTS_CLIENT: (i32, i32) = (290, 36);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Attributes,
    Stretch,
    Flip,
}

impl Kind {
    pub fn title(self) -> String {
        match self {
            Kind::Attributes => tr!("title-attributes"),
            Kind::Stretch => tr!("title-stretch"),
            Kind::Flip => tr!("title-flip"),
        }
    }

    pub fn client(self) -> (i32, i32) {
        match self {
            Kind::Attributes => (230, 80),
            Kind::Stretch => (270, 210),
            Kind::Flip => (270, 170),
        }
    }
}

fn edit(id: &'static str, x: i32, y: i32, w: i32, value: &str) -> Item {
    Item::edit(id, Rect::new(x, y, w, 20), value, true)
}

/// OK and Cancel down the right edge (`side`) or along the bottom (`bottom`).
fn buttons(client: (i32, i32), side: bool) -> [Item; 2] {
    let (w, h) = client;
    let (ok, cancel) = if side {
        ((w - BUTTON_W - 8, 8), (w - BUTTON_W - 8, 8 + BUTTON_H + 6))
    } else {
        (
            (w - 2 * BUTTON_W - 14, h - BUTTON_H - 8),
            (w - BUTTON_W - 8, h - BUTTON_H - 8),
        )
    };
    [
        Item::button(
            OK,
            Rect::new(ok.0, ok.1, BUTTON_W, BUTTON_H),
            &reprisekit::kit_tr!("ok"),
            true,
            true,
        ),
        Item::button(
            CANCEL,
            Rect::new(cancel.0, cancel.1, BUTTON_W, BUTTON_H),
            &reprisekit::kit_tr!("cancel"),
            true,
            false,
        ),
    ]
}

pub fn attributes_form(w: i32, h: i32) -> Form {
    let mut items = vec![
        Item::label_at("width-label", 10, 15, 56, &tr!("attr-width")),
        edit(WIDTH, 68, 12, 60, &w.to_string()),
        Item::label_at("height-label", 10, 45, 56, &tr!("attr-height")),
        edit(HEIGHT, 68, 42, 60, &h.to_string()),
    ];
    items.extend(buttons(Kind::Attributes.client(), true));
    Form::new(items)
}

pub fn stretch_form() -> Form {
    let rows = |y: i32, ids: [&'static str; 4], labels: [String; 2], value: &str, unit: &str| {
        [
            Item::label_at(ids[2], 22, y + 3, 84, &labels[0]),
            edit(ids[0], 110, y, 50, value),
            Item::label_at("", 166, y + 3, 80, unit),
            Item::label_at(ids[3], 22, y + 31, 84, &labels[1]),
            edit(ids[1], 110, y + 28, 50, value),
            Item::label_at("", 166, y + 31, 80, unit),
        ]
    };
    let mut items = vec![Item::group(
        "resize",
        Rect::new(8, 6, 254, 80),
        &tr!("stretch-resize"),
    )];
    items.extend(rows(
        26,
        [RESIZE_H, RESIZE_V, "resize-h-label", "resize-v-label"],
        [tr!("stretch-horizontal"), tr!("stretch-vertical")],
        "100",
        &tr!("stretch-percent"),
    ));
    items.push(Item::group(
        "skew",
        Rect::new(8, 92, 254, 80),
        &tr!("stretch-skew"),
    ));
    items.extend(rows(
        112,
        [SKEW_H, SKEW_V, "skew-h-label", "skew-v-label"],
        [tr!("stretch-skew-horizontal"), tr!("stretch-skew-vertical")],
        "0",
        &tr!("stretch-degrees"),
    ));
    items.extend(buttons(Kind::Stretch.client(), false));
    Form::new(items)
}

pub fn flip_form() -> Form {
    let labels = [
        tr!("flip-horizontal"),
        tr!("flip-vertical"),
        tr!("rotate-90"),
        tr!("rotate-180"),
        tr!("rotate-270"),
    ];
    let mut items = vec![Item::group(
        "flips",
        Rect::new(8, 6, 254, 124),
        &tr!("flip-group"),
    )];
    for (i, (id, label)) in FLIPS.iter().zip(labels).enumerate() {
        items.push(Item::radio(id, 22, 26 + 20 * i as i32, &label, "flip").checked(i == 0));
    }
    items.extend(buttons(Kind::Flip.client(), false));
    Form::new(items)
}

pub fn fonts_form(families: &[String], font: &textbox::Font) -> Form {
    let names: Vec<&str> = families.iter().map(String::as_str).collect();
    let at = families.iter().position(|f| *f == font.family).unwrap_or(0);
    let sizes: Vec<String> = textbox::SIZES.iter().map(u32::to_string).collect();
    let sizes: Vec<&str> = sizes.iter().map(String::as_str).collect();
    let size_at = textbox::SIZES
        .iter()
        .position(|&s| s == font.px)
        .unwrap_or(0);
    Form::new(vec![
        Item::new(
            FAMILY,
            Rect::new(8, 7, 160, reprisekit::controls::combo::COMBO_H),
            Control::Combo(Combo::new(&names, at, 12)),
        ),
        Item::new(
            SIZE,
            Rect::new(174, 7, 48, reprisekit::controls::combo::COMBO_H),
            Control::Combo(Combo::new(&sizes, size_at, 12)),
        ),
        Item::check(BOLD, 230, 10, &tr!("fonts-bold")).checked(font.bold),
    ])
}

/// A whole number in `lo..=hi`.
fn number(f: &Form, id: &str, lo: i32, hi: i32) -> Option<i32> {
    f.edit_text(id)
        .trim()
        .parse::<i32>()
        .ok()
        .filter(|n| (lo..=hi).contains(n))
}

impl Paint {
    pub(crate) fn error(&mut self, text: String) {
        let b = MsgBox::new(
            tr!("name"),
            msgbox::Fields {
                text,
                icon: self.warn_icon.clone(),
                buttons: vec![reprisekit::kit_tr!("ok")],
                default: 0,
                min_text_h: None,
                min_w: 0,
                max_w: repriseapp::dialog_max_w(),
            },
        );
        self.prompt = Some((b, Prompt::Error));
    }

    pub(crate) fn ask(&mut self, text: String, buttons: Vec<String>, default: usize, then: Prompt) {
        let b = MsgBox::new(
            tr!("name"),
            msgbox::Fields {
                text,
                icon: self.warn_icon.clone(),
                buttons,
                default,
                min_text_h: None,
                min_w: 0,
                max_w: repriseapp::dialog_max_w(),
            },
        );
        self.prompt = Some((b, then));
    }

    pub(crate) fn prompt_event(&mut self, ev: Event) -> Reply {
        let Some((b, _)) = &mut self.prompt else {
            return Reply::none();
        };
        match b.event(&ev, &self.dialog_text, &self.forms) {
            oneshot::Outcome::Closed(answer) => {
                let Some((_, what)) = self.prompt.take() else {
                    return Reply::redraw_all();
                };
                self.answered(what, answer)
            }
            oneshot::Outcome::Redraw => Reply::redraw(),
            oneshot::Outcome::None => Reply::none(),
        }
    }

    pub(crate) fn open_dialog(&mut self, kind: Kind) {
        self.commit_text();
        self.doc.finish();
        let f = match kind {
            Kind::Attributes => attributes_form(self.doc.img.w, self.doc.img.h),
            Kind::Stretch => stretch_form(),
            Kind::Flip => flip_form(),
        };
        self.dialog = Some((kind, f));
    }

    pub(crate) fn dialog_event(&mut self, ev: Event, ctx: &mut Ctx) -> Reply {
        if ev == Event::CloseRequest {
            self.dialog = None;
            return Reply::redraw_all();
        }
        let Some((kind, f)) = &mut self.dialog else {
            return Reply::none();
        };
        let kind = *kind;
        match repriseapp::keys::form_input(f, &ev, ctx) {
            Some(form::Event::Cancel | form::Event::Pressed(CANCEL)) => {
                self.dialog = None;
                Reply::redraw_all()
            }
            Some(form::Event::Default | form::Event::Pressed(OK)) => {
                if self.apply(kind) {
                    self.dialog = None;
                    self.hold_scroll(ctx);
                }
                Reply::redraw_all()
            }
            Some(form::Event::None) | None => Reply::none(),
            Some(_) => Reply::redraw(),
        }
    }

    /// The dialog's OK: the picture changed as one step. False when a field is out of range.
    fn apply(&mut self, kind: Kind) -> bool {
        let Some((_, f)) = &self.dialog else {
            return false;
        };
        let img = &self.doc.img;
        let out = match kind {
            Kind::Attributes => {
                match (
                    number(f, WIDTH, 1, MAX_SIDE),
                    number(f, HEIGHT, 1, MAX_SIDE),
                ) {
                    (Some(w), Some(h)) => Some(img.with_size(w, h, self.doc.secondary)),
                    _ => None,
                }
            }
            Kind::Stretch => {
                let fields = (
                    number(f, RESIZE_H, 1, 500),
                    number(f, RESIZE_V, 1, 500),
                    number(f, SKEW_H, -89, 89),
                    number(f, SKEW_V, -89, 89),
                );
                match fields {
                    (Some(rh), Some(rv), Some(sh), Some(sv)) => {
                        let w = (img.w * rh / 100).clamp(1, MAX_SIDE);
                        let h = (img.h * rv / 100).clamp(1, MAX_SIDE);
                        let mut out = img.resized(w, h);
                        if (sh, sv) != (0, 0) {
                            out = out.skewed(sh, sv, self.doc.secondary);
                        }
                        Some(out)
                    }
                    _ => None,
                }
            }
            Kind::Flip => {
                let pick = FLIPS.iter().position(|id| f.checked(id) == Some(true));
                Some(match pick {
                    Some(1) => img.flipped_v(),
                    Some(2) => img.rotated(1),
                    Some(3) => img.rotated(2),
                    Some(4) => img.rotated(3),
                    _ => img.flipped_h(),
                })
            }
        };
        match out {
            Some(img) => {
                self.doc.replace(img);
                true
            }
            None => {
                let max = if kind == Kind::Attributes {
                    MAX_SIDE
                } else {
                    500
                };
                self.error(tr!("bad-size", max = max));
                false
            }
        }
    }

    pub(crate) fn open_fonts(&mut self) {
        if self.families.is_empty() {
            self.families = reprisekit::text::families();
        }
        if !self.families.contains(&self.text.font.family)
            && let Some(first) = self.families.first()
        {
            self.text.font.family = first.clone();
        }
        self.fonts = Some(fonts_form(&self.families, &self.text.font));
    }

    pub(crate) fn fonts_event(&mut self, ev: Event, ctx: &mut Ctx) -> Reply {
        if ev == Event::CloseRequest {
            self.fonts = None;
            return Reply::redraw_all();
        }
        let Some(f) = &mut self.fonts else {
            return Reply::none();
        };
        match repriseapp::keys::form_input(f, &ev, ctx) {
            Some(form::Event::Changed(_)) => {
                if let Some(i) = f.combo_selected(FAMILY)
                    && let Some(name) = self.families.get(i)
                {
                    self.text.font.family = name.clone();
                }
                if let Some(i) = f.combo_selected(SIZE) {
                    self.text.font.px = textbox::SIZES[i];
                }
                self.text.font.bold = f.checked(BOLD) == Some(true);
                Reply {
                    redraw: true,
                    redraw_wins: vec![MAIN],
                    ..Reply::none()
                }
            }
            Some(form::Event::None) | None => Reply::none(),
            Some(_) => Reply::redraw(),
        }
    }

    pub(crate) fn about_event(&mut self, ev: Event) -> Reply {
        match oneshot::event(&ev, &self.dialog_text, &self.forms, || self.about_form()) {
            oneshot::Step::Closed
            | oneshot::Step::Out(
                form::Event::Cancel | form::Event::Default | form::Event::Pressed(aboutbox::OK),
            ) => {
                self.about = false;
                Reply::redraw_all()
            }
            oneshot::Step::Redraw => Reply::redraw(),
            oneshot::Step::Out(form::Event::None) | oneshot::Step::None => Reply::none(),
            oneshot::Step::Out(_) => Reply::redraw(),
        }
    }
}
