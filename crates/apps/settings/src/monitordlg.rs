//! No is the default and has the focus, so Enter and Escape both go back.

use reprisekit::classic::widgets::{BUTTON_H, BUTTON_W};
use reprisekit::controls::form::{self, Form, Item};
use reprisekit::raster::{Canvas, Rect};

pub const CLIENT: (i32, i32) = (345, 110);
pub const ICON: &str = "icon";
pub const COUNTDOWN: &str = "countdown";
pub const YES: &str = "yes";
pub const NO: &str = "no";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    None,
    Redraw,
    Keep,
    Revert,
}

pub struct MonitorDialog {
    pub form: Form,
    pub left: u32,
}

/// "Reverting in 15 seconds"; "1 second" at 1.
pub fn countdown_text(left: u32) -> String {
    let unit = if left == 1 { "second" } else { "seconds" };
    format!("Reverting in {left} {unit}")
}

impl MonitorDialog {
    pub fn new(seconds: u32, icon: Option<Canvas>) -> MonitorDialog {
        let items = vec![
            Item::picture(ICON, Rect::new(7, 12, 32, 32), icon),
            Item::lines(
                "text",
                Rect::new(50, 12, 288, 26),
                &[
                    "Your desktop has been reconfigured.  Do you want to keep",
                    "these settings?",
                ],
            ),
            Item::label_at(COUNTDOWN, 11, 76, 160, &countdown_text(seconds)),
            Item::button(
                YES,
                Rect::new(182, 64, BUTTON_W, BUTTON_H),
                "&Yes",
                true,
                false,
            ),
            Item::button(
                NO,
                Rect::new(263, 64, BUTTON_W, BUTTON_H),
                "&No",
                true,
                true,
            ),
        ];
        let mut form = Form::new(items);
        form.focus_id(NO);
        MonitorDialog {
            form,
            left: seconds,
        }
    }

    pub fn set_left(&mut self, left: u32) -> bool {
        if left == self.left {
            return false;
        }
        self.left = left;
        self.form.set_text(COUNTDOWN, &countdown_text(left));
        true
    }

    pub fn event(&self, ev: form::Event) -> Outcome {
        match ev {
            form::Event::Pressed(YES) => Outcome::Keep,
            form::Event::Pressed(NO) | form::Event::Default | form::Event::Cancel => {
                Outcome::Revert
            }
            form::Event::Redraw => Outcome::Redraw,
            _ => Outcome::None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reprisekit::text::TextRenderer;
    use reprisekit::theme::{Color, Theme};

    #[test]
    fn yes_keeps_and_everything_else_goes_back() {
        let d = MonitorDialog::new(15, None);
        let key = |k| {
            let mut f = d.form.clone();
            d.event(f.key(k))
        };
        assert_eq!(key(form::Key::Access('y')), Outcome::Keep);
        assert_eq!(key(form::Key::Access('n')), Outcome::Revert);
        assert_eq!(key(form::Key::Enter), Outcome::Revert, "No is the default");
        assert_eq!(key(form::Key::Escape), Outcome::Revert);
    }

    #[test]
    fn the_countdown_says_how_long_is_left() {
        let mut d = MonitorDialog::new(15, None);
        assert!(!d.set_left(15), "unchanged");
        assert!(d.set_left(14));
        match &d.form.item(COUNTDOWN).unwrap().control {
            form::Control::Label { text, .. } => assert_eq!(text, "Reverting in 14 seconds"),
            other => panic!("{other:?}"),
        }
        assert_eq!(countdown_text(1), "Reverting in 1 second");
        assert_eq!(countdown_text(0), "Reverting in 0 seconds");
    }

    #[test]
    fn the_question_draws_no_as_the_default_beside_its_countdown() {
        let theme = Theme::default();
        let text = TextRenderer::load(&theme.fonts);
        let red = Color::rgb(255, 0, 0);
        let mut icon = Canvas::new(32, 32);
        icon.fill_rect(icon.bounds(), red);
        let d = MonitorDialog::new(15, Some(icon));
        let mut c = Canvas::new(CLIENT.0 as u32, CLIENT.1 as u32);
        c.fill_rect(c.bounds(), theme.colors.button_face);
        form::draw(
            &mut c,
            &text,
            &theme.paint(),
            &d.form,
            false,
            theme.fonts.size_px,
        );
        let ink = theme.colors.button_text.argb();
        assert_eq!(c.get(263, 64), ink, "No's default ring");
        assert_ne!(c.get(182, 64), ink, "Yes has none");
        assert_eq!(c.get(23, 28), red.argb(), "the icon");
        let r = d.form.item(COUNTDOWN).unwrap().rect;
        assert!((r.y..r.bottom()).any(|y| (r.x..r.right()).any(|x| c.get(x, y) == ink)));
        for it in &d.form.items {
            let r = it.rect;
            assert!(
                r.x >= 0 && r.y >= 0 && r.right() <= CLIENT.0 && r.bottom() <= CLIENT.1,
                "{}",
                it.id
            );
        }
    }
}
