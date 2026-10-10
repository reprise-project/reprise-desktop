use reprisekit::classic::widgets::{BUTTON_GAP, BUTTON_H, BUTTON_W};
use reprisekit::controls::form::{self, Control, Form, Item};
use reprisekit::controls::updown::{self, UpDown};
use reprisekit::raster::Rect;
use reprisekit::saver::{Kind, Options};
use reprisekit::textedit::TextEdit;
use reprisekit::trackbar::{self, TrackBar};

use crate::model::Settings;

const W: i32 = 300;
const MARGIN: i32 = 12;
const FIELD_X: i32 = MARGIN + 104;

pub const OK: &str = "ok";
pub const CANCEL: &str = "cancel";
const SPEED: &str = "speed";
const COUNT: &str = "count";
const CYCLE: &str = "cycle";
const FOLDER: &str = "folder";
const SECONDS: &str = "seconds";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    None,
    Redraw,
    Accept,
    Cancel,
}

pub struct SaverDialog {
    pub form: Form,
    pub kind: Kind,
    pub client: (i32, i32),
}

impl SaverDialog {
    pub fn new(kind: Kind, s: &Settings) -> SaverDialog {
        let o = crate::saverpage::options(s);
        let mut items = Vec::new();
        let mut y = MARGIN + 4;
        let label = |id, y, text| Item::label_at(id, MARGIN, y, FIELD_X - MARGIN - 4, text);
        let spin = |id, y: i32, value: u32, (lo, hi): (u32, u32)| {
            Item::new(
                id,
                Rect::new(FIELD_X, y - 4, updown::WIDTH, updown::HEIGHT),
                Control::UpDown(UpDown::new(value as i32, lo as i32, hi as i32)),
            )
        };
        if kind != Kind::Photos {
            items.push(label("speed_label", y, "Spee&d:"));
            items.push(Item::label_at("slow", FIELD_X, y, 26, "Slow"));
            let steps = (Options::SPEED.1 - Options::SPEED.0 + 1) as usize;
            items.push(Item::new(
                SPEED,
                Rect::new(FIELD_X + 28, y - 6, 110, trackbar::HEIGHT),
                Control::TrackBar(TrackBar::new((o.speed - Options::SPEED.0) as usize, steps)),
            ));
            items.push(Item::label_at("fast", FIELD_X + 142, y, 26, "Fast"));
            y += 36;
        }
        match kind {
            Kind::Starfield => {
                items.push(label("count_label", y, "&Stars:"));
                items.push(spin(COUNT, y, o.stars, Options::STARS));
                y += 28;
            }
            Kind::Mystify => {
                items.push(label("count_label", y, "S&hapes:"));
                items.push(spin(COUNT, y, o.shapes, Options::SHAPES));
                y += 28;
                items
                    .push(Item::check(CYCLE, MARGIN, y, "&Change colours").checked(o.cycle_colors));
                y += 24;
            }
            Kind::Logo => {
                items.push(
                    Item::check(CYCLE, MARGIN, y, "&Change colour at each bounce")
                        .checked(o.cycle_colors),
                );
                y += 24;
            }
            Kind::Photos => {
                items.push(Item::label_at(
                    "folder_label",
                    MARGIN,
                    y,
                    W - 2 * MARGIN,
                    "&Folder (empty for Pictures):",
                ));
                items.push(Item::new(
                    FOLDER,
                    Rect::new(MARGIN, y + 16, W - 2 * MARGIN, 20),
                    Control::Edit {
                        edit: TextEdit::new(s.saver_folder.clone(), false),
                        first: Default::default(),
                        enabled: true,
                    },
                ));
                y += 50;
                items.push(label("seconds_label", y, "&Show each for:"));
                items.push(spin(SECONDS, y, o.photo_seconds, Options::PHOTO_SECONDS));
                items.push(Item::label_at(
                    "seconds_units",
                    FIELD_X + updown::WIDTH + 6,
                    y,
                    60,
                    "seconds",
                ));
                y += 28;
            }
            Kind::Blank => {}
        }
        let h = y + 8 + BUTTON_H + MARGIN;
        let cancel_x = W - MARGIN - BUTTON_W;
        let by = h - MARGIN - BUTTON_H;
        items.push(Item::button(
            OK,
            Rect::new(cancel_x - BUTTON_GAP - BUTTON_W, by, BUTTON_W, BUTTON_H),
            "OK",
            true,
            true,
        ));
        items.push(Item::button(
            CANCEL,
            Rect::new(cancel_x, by, BUTTON_W, BUTTON_H),
            "Cancel",
            true,
            false,
        ));
        let mut form = Form::new(items);
        form.set_focus(Some(0));
        form.focus_id(if kind == Kind::Photos { FOLDER } else { SPEED });
        SaverDialog {
            form,
            kind,
            client: (W, h),
        }
    }

    pub fn title(&self) -> String {
        format!("{} Settings", self.kind.label())
    }

    pub fn event(&mut self, ev: form::Event) -> Outcome {
        match ev {
            form::Event::Pressed(OK) | form::Event::Default => Outcome::Accept,
            form::Event::Pressed(CANCEL) | form::Event::Cancel => Outcome::Cancel,
            form::Event::Redraw | form::Event::Changed(_) => Outcome::Redraw,
            _ => Outcome::None,
        }
    }

    /// Put the dialog's values into `s`: only the keys this saver has.
    pub fn apply(&self, s: &mut Settings) {
        let f = &self.form;
        if let Some(pos) = f.track_pos(SPEED) {
            s.saver_speed = pos as i64 + Options::SPEED.0 as i64;
        }
        let num = |id| match f.item(id).map(|i| &i.control) {
            Some(Control::UpDown(u)) => Some(u.value() as i64),
            _ => None,
        };
        if let Some(n) = num(COUNT) {
            match self.kind {
                Kind::Starfield => s.saver_stars = n,
                Kind::Mystify => s.saver_shapes = n,
                _ => {}
            }
        }
        if let Some(on) = f.checked(CYCLE) {
            s.saver_cycle = on;
        }
        if let Some(Control::Edit { edit, .. }) = f.item(FOLDER).map(|i| &i.control) {
            s.saver_folder = edit.text.trim().to_string();
        }
        if let Some(n) = num(SECONDS) {
            s.saver_photo_seconds = n;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_saver_shows_its_own_rows() {
        let s = Settings::default();
        let ids = |k| {
            SaverDialog::new(k, &s)
                .form
                .items
                .iter()
                .map(|i| i.id)
                .filter(|id| [SPEED, COUNT, CYCLE, FOLDER, SECONDS].contains(id))
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(Kind::Starfield), [SPEED, COUNT]);
        assert_eq!(ids(Kind::Mystify), [SPEED, COUNT, CYCLE]);
        assert_eq!(ids(Kind::Logo), [SPEED, CYCLE]);
        assert_eq!(ids(Kind::Photos), [FOLDER, SECONDS]);
    }

    #[test]
    fn ok_writes_only_the_savers_own_keys() {
        let s = Settings::default();
        let mut d = SaverDialog::new(Kind::Mystify, &s);
        d.form.set_track(SPEED, 9);
        if let Some(Control::UpDown(u)) = d.form.item_mut(COUNT).map(|i| &mut i.control) {
            u.set(4);
        }
        d.form.set_check(CYCLE, false);
        let mut got = s.clone();
        d.apply(&mut got);
        assert_eq!(
            (got.saver_speed, got.saver_shapes, got.saver_cycle),
            (10, 4, false)
        );
        assert_eq!(got.saver_stars, s.saver_stars, "Starfield's count stays");
        assert_eq!(d.event(form::Event::Default), Outcome::Accept);
        assert_eq!(d.event(form::Event::Cancel), Outcome::Cancel);
    }

    #[test]
    fn everything_sits_inside_the_client() {
        let s = Settings::default();
        for k in [Kind::Starfield, Kind::Mystify, Kind::Logo, Kind::Photos] {
            let d = SaverDialog::new(k, &s);
            for i in &d.form.items {
                let r = i.rect;
                assert!(
                    r.x >= 0 && r.y >= 0 && r.right() <= d.client.0 && r.bottom() <= d.client.1,
                    "{k:?} {} at {r:?}",
                    i.id
                );
            }
        }
    }
}
