//! "Create Hotspot": turns a Wi-Fi radio into an access point sharing this computer's connection. Asks for the network's name, its password (shown), WPA2 or WPA3, and the band.

use reprisekit::classic::widgets::{BUTTON_GAP, BUTTON_H, BUTTON_W};
use reprisekit::controls::form::{self, Control, Form, Item};

use crate::nm::{Band, Psk};
use crate::vpnsheet::text_of;

pub const CLIENT: (i32, i32) = (360, 214);

const MARGIN: i32 = 12;
const LABEL_W: i32 = 96;
const FIELD_X: i32 = 110;
const FIELD_W: i32 = CLIENT.0 - MARGIN - FIELD_X;
const FIELD_H: i32 = 20;
const ROW_Y: [i32; 4] = [40, 70, 100, 130];
const LABEL_DY: i32 = 4;
const BTN_Y: i32 = CLIENT.1 - MARGIN - BUTTON_H;
const CANCEL_X: i32 = CLIENT.0 - MARGIN - BUTTON_W;
const START_X: i32 = CANCEL_X - BUTTON_GAP - BUTTON_W;

const SECURITY: [&str; 2] = ["WPA2 Personal", "WPA3 Personal"];
const BANDS: [&str; 3] = ["Automatic", "2.4 GHz", "5 GHz"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    None,
    Redraw,
    Start {
        ssid: String,
        psk: Psk,
        wpa3: bool,
        band: Band,
    },
    Cancel,
}

/// Whether `ssid` and `password` make a network: a name of 1 to 32 bytes and a passphrase of 8 to 63, which both WPA2 and WPA3 clients take.
pub fn valid(ssid: &str, password: &str) -> bool {
    (1..=32).contains(&ssid.len()) && (8..=63).contains(&password.len())
}

/// The dialog's model; the host owns the window. `device` is the radio it was opened for.
pub struct HotspotDialog {
    pub device: String,
    pub form: Form,
}

impl HotspotDialog {
    pub fn new(device: &str, ssid: &str) -> HotspotDialog {
        let field = |id, y, text: &str| {
            Item::edit(
                id,
                reprisekit::raster::Rect::new(FIELD_X, y, FIELD_W, FIELD_H),
                text,
                true,
            )
        };
        let label = |id, i: usize, text: &str| {
            Item::label_at(id, MARGIN, ROW_Y[i] + LABEL_DY, LABEL_W, text)
        };
        let items = vec![
            Item::lines_at(
                "intro",
                MARGIN,
                MARGIN,
                CLIENT.0 - 2 * MARGIN,
                &["Share this computer's connection with other devices over Wi-Fi."],
            ),
            label("ssid_l", 0, "Network &name:"),
            field("ssid", ROW_Y[0], ssid),
            label("password_l", 1, "&Password:"),
            field("password", ROW_Y[1], ""),
            label("security_l", 2, "&Security:"),
            Item::combo("security", FIELD_X, ROW_Y[2], FIELD_W, &SECURITY, 2),
            label("band_l", 3, "&Band:"),
            Item::combo("band", FIELD_X, ROW_Y[3], FIELD_W, &BANDS, 3),
            Item::lines_at(
                "hint",
                MARGIN,
                ROW_Y[3] + 30,
                CLIENT.0 - 2 * MARGIN,
                &["The password takes 8 to 63 characters."],
            ),
            Item::button(
                "start",
                reprisekit::raster::Rect::new(START_X, BTN_Y, BUTTON_W, BUTTON_H),
                "&Start",
                false,
                true,
            ),
            Item::button_at("cancel", CANCEL_X, BTN_Y, BUTTON_W, "Cancel", true),
        ];
        let mut d = HotspotDialog {
            device: device.to_string(),
            form: Form::new(items),
        };
        d.form.focus_id("password");
        d.sync_button();
        d
    }

    pub fn window_title() -> String {
        "Create Hotspot".into()
    }

    fn ssid(&self) -> &str {
        text_of(&self.form, "ssid")
    }

    fn password(&self) -> &str {
        text_of(&self.form, "password")
    }

    fn sync_button(&mut self) {
        let ok = valid(self.ssid(), self.password());
        let focused = self.form.focus() == self.form.index_of("start");
        if let Some(Control::Button { enabled, .. }) =
            self.form.item_mut("start").map(|i| &mut i.control)
        {
            *enabled = ok;
        }
        if focused && !ok {
            self.form.focus_edge(false);
        }
    }

    pub fn event(&mut self, ev: form::Event) -> Outcome {
        match ev {
            form::Event::Changed(_) => {
                self.sync_button();
                Outcome::Redraw
            }
            form::Event::Pressed("start") | form::Event::Default => {
                if !valid(self.ssid(), self.password()) {
                    return Outcome::None;
                }
                Outcome::Start {
                    ssid: self.ssid().to_string(),
                    psk: Psk::new(self.password()),
                    wpa3: self.form.combo_selected("security") == Some(1),
                    band: match self.form.combo_selected("band") {
                        Some(1) => Band::Bg,
                        Some(2) => Band::A,
                        _ => Band::Auto,
                    },
                }
            }
            form::Event::Pressed("cancel") | form::Event::Cancel => Outcome::Cancel,
            form::Event::Redraw => Outcome::Redraw,
            _ => Outcome::None,
        }
    }
}

pub fn default_ssid() -> String {
    std::fs::read_to_string("/etc/hostname")
        .ok()
        .map(|h| h.trim().to_string())
        .filter(|h| !h.is_empty() && h.len() <= 32)
        .unwrap_or_else(|| "Reprise".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use reprisekit::raster::Canvas;

    fn type_into(d: &mut HotspotDialog, id: &str, s: &str) {
        d.form.focus_id(id);
        for ch in s.chars() {
            let ev = d.form.key(form::Key::Char(ch));
            d.event(ev);
        }
    }

    fn start_enabled(d: &HotspotDialog) -> bool {
        matches!(
            d.form.item("start").unwrap().control,
            Control::Button { enabled: true, .. }
        )
    }

    #[test]
    fn start_waits_for_a_long_enough_password() {
        let mut d = HotspotDialog::new("/dev/3", "desk");
        assert!(!start_enabled(&d));
        assert_eq!(d.event(form::Event::Default), Outcome::None);
        type_into(&mut d, "password", "1234567");
        assert!(!start_enabled(&d), "seven characters is one short");
        type_into(&mut d, "password", "8");
        assert!(start_enabled(&d));
        assert_eq!(
            d.event(form::Event::Pressed("start")),
            Outcome::Start {
                ssid: "desk".into(),
                psk: Psk::new("12345678"),
                wpa3: false,
                band: Band::Auto,
            }
        );
    }

    #[test]
    fn the_combos_pick_wpa3_and_the_band() {
        let mut d = HotspotDialog::new("/dev/3", "desk");
        type_into(&mut d, "password", "correct horse");
        d.form.set_combo("security", 1);
        d.form.set_combo("band", 2);
        assert!(matches!(
            d.event(form::Event::Default),
            Outcome::Start {
                wpa3: true,
                band: Band::A,
                ..
            }
        ));
        assert_eq!(d.event(form::Event::Cancel), Outcome::Cancel);
    }

    #[test]
    fn a_name_takes_one_to_32_bytes() {
        assert!(valid("a", "12345678"));
        assert!(!valid("", "12345678"));
        assert!(!valid(&"x".repeat(33), "12345678"));
        assert!(!valid("a", &"x".repeat(64)));
    }

    #[test]
    fn the_dialog_draws_its_rows() {
        let theme = reprisekit::theme::Theme::default();
        let text = reprisekit::text::TextRenderer::load(&reprisekit::theme::FontConfig::default());
        let d = HotspotDialog::new("/dev/3", "desk");
        let mut c = Canvas::new(CLIENT.0 as u32, CLIENT.1 as u32);
        c.fill_rect(c.bounds(), theme.colors.button_face);
        form::draw(
            &mut c,
            &text,
            &theme.paint(),
            &d.form,
            true,
            theme.fonts.size_px,
        );
        let rgb = |v: u32| v & 0xffffff;
        let ssid = d.form.item("ssid").unwrap().rect;
        assert_eq!(
            rgb(c.get(ssid.x, ssid.y)),
            rgb(theme.colors.button_shadow.argb())
        );
        assert_eq!(
            rgb(c.get(ssid.x + 3, ssid.y + 3)),
            rgb(theme.colors.window.argb())
        );
        let start = d.form.item("start").unwrap().rect;
        let greyed = rgb(theme.colors.gray_text.argb());
        assert!(
            (start.x..start.right())
                .flat_map(|x| (start.y..start.bottom()).map(move |y| (x, y)))
                .any(|(x, y)| rgb(c.get(x, y)) == greyed),
            "Start's label is greyed"
        );
        for r in [start, d.form.item("cancel").unwrap().rect] {
            assert!(r.right() <= CLIENT.0 - MARGIN && r.bottom() <= CLIENT.1 - MARGIN);
        }
    }

    #[test]
    #[ignore]
    fn render_hotspot() {
        let theme = reprisekit::theme::Theme::default();
        let text = reprisekit::text::TextRenderer::load(&reprisekit::theme::FontConfig::default());
        let mut d = HotspotDialog::new("/dev/3", "desk");
        type_into(&mut d, "password", "correct horse");
        let mut c = Canvas::new(CLIENT.0 as u32, CLIENT.1 as u32);
        c.fill_rect(c.bounds(), theme.colors.button_face);
        form::draw(
            &mut c,
            &text,
            &theme.paint(),
            &d.form,
            true,
            theme.fonts.size_px,
        );
        reprisekit::raster::write_preview(&c, "create-hotspot.png");
    }
}
