pub mod eap;

use std::cell::Cell;

use reprisekit::classic::widgets::{BUTTON_GAP, BUTTON_H, BUTTON_W};
use reprisekit::controls::form::{self, Control, Form, Item};
use reprisekit::raster::Rect;
use reprisekit::textedit::TextEdit;

use crate::nm::{Psk, Security, Ssid};

pub const CLIENT: (i32, i32) = (380, 220);

const MARGIN: i32 = 12;
const INTRO_Y: i32 = MARGIN;
const INTRO_W: i32 = CLIENT.0 - 2 * MARGIN;
const INTRO_COLS: usize = 62;

const LABEL_X: i32 = MARGIN;
const LABEL_W: i32 = 124;
const FIELD_X: i32 = 140;
const FIELD_W: i32 = CLIENT.0 - MARGIN - FIELD_X;
const FIELD_H: i32 = 20;
const ROW_Y: [i32; 2] = [112, 140];
const LABEL_DY: i32 = 4;

const BTN_Y: i32 = CLIENT.1 - MARGIN - BUTTON_H;
const CANCEL_X: i32 = CLIENT.0 - MARGIN - BUTTON_W;
const CONNECT_X: i32 = CANCEL_X - BUTTON_GAP - BUTTON_W;

/// The hidden-network dialog: the name and security rows above the two key rows.
pub const HIDDEN_CLIENT: (i32, i32) = (CLIENT.0, 204);
const HIDDEN_ROW_Y: [i32; 4] = [48, 76, 104, 132];
const HIDDEN_INTRO: [&str; 2] = [
    "Type the name of a network that does not broadcast it, and choose",
    "the security it uses.",
];
const HIDDEN_SECURITY: [&str; 3] = ["Open", "WPA-PSK", "Enterprise (802.1X)"];
const HIDDEN_KINDS: [Security; 3] = [Security::Open, Security::Psk, Security::Enterprise];
/// The longest SSID in bytes, 802.11's limit.
const SSID_MAX: usize = 32;

/// The shortest and longest WPA-PSK passphrase in bytes, the unit
/// NetworkManager checks `psk` in.
const KEY_LEN: std::ops::RangeInclusive<usize> = 8..=63;

fn key_ok(key: &str, security: Security) -> bool {
    match security {
        Security::Sae => !key.is_empty(),
        _ => {
            let n = key.len();
            // 64 hex digits is a raw PSK rather than a passphrase.
            KEY_LEN.contains(&n) || (n == 64 && key.chars().all(|c| c.is_ascii_hexdigit()))
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    None,
    Redraw,
    /// Join with this key, None for an open hidden network.
    Connect(Option<Psk>),
    /// A hidden 802.1X network: its login goes on in the enterprise dialog.
    Enterprise,
    Cancel,
}

/// The key dialog for a network in the list, or for a hidden one (`hidden`), which adds the
/// network's name and a choice of security above the key rows.
pub struct KeyDialog {
    ssid: Ssid,
    pub ap: String,
    security: Security,
    pub hidden: bool,
    pub form: Form,
}

impl KeyDialog {
    pub fn new(ssid: &Ssid, ap: &str, security: Security) -> KeyDialog {
        let intro = intro_lines(&ssid.to_string());
        let refs: Vec<&str> = intro.iter().map(|s| s.as_str()).collect();
        let items = vec![
            Item::lines_at("intro", MARGIN, INTRO_Y, INTRO_W, &refs),
            Item::label_at(
                "key_l",
                LABEL_X,
                ROW_Y[0] + LABEL_DY,
                LABEL_W,
                "Network &key:",
            ),
            field("key", ROW_Y[0]),
            Item::label_at(
                "confirm_l",
                LABEL_X,
                ROW_Y[1] + LABEL_DY,
                LABEL_W,
                "C&onfirm network key:",
            ),
            field("confirm", ROW_Y[1]),
            connect_button(false),
            Item::button_at("cancel", CANCEL_X, BTN_Y, BUTTON_W, "Cancel", true),
        ];
        KeyDialog {
            ssid: ssid.clone(),
            ap: ap.into(),
            security,
            hidden: false,
            form: Form::new(items),
        }
    }

    /// The dialog for a network that is in no scan, joined through no access point ("/").
    pub fn hidden() -> KeyDialog {
        let label = |id, i: usize, text: &str| {
            Item::label_at(id, LABEL_X, HIDDEN_ROW_Y[i] + LABEL_DY, LABEL_W, text)
        };
        let key = field("key", HIDDEN_ROW_Y[2]);
        let confirm = field("confirm", HIDDEN_ROW_Y[3]);
        let btn_y = HIDDEN_CLIENT.1 - MARGIN - BUTTON_H;
        let mut connect = connect_button(false);
        connect.rect.y = btn_y;
        let items = vec![
            Item::lines_at("intro", MARGIN, INTRO_Y, INTRO_W, &HIDDEN_INTRO),
            label("ssid_l", 0, "Network &name (SSID):"),
            Item::edit(
                "ssid",
                Rect::new(FIELD_X, HIDDEN_ROW_Y[0], FIELD_W, FIELD_H),
                "",
                true,
            ),
            label("security_l", 1, "&Security:"),
            Item::combo(
                "security",
                FIELD_X,
                HIDDEN_ROW_Y[1],
                FIELD_W,
                &HIDDEN_SECURITY,
                HIDDEN_SECURITY.len(),
            ),
            label("key_l", 2, "Network &key:"),
            key,
            label("confirm_l", 3, "C&onfirm network key:"),
            confirm,
            connect,
            Item::button_at("cancel", CANCEL_X, btn_y, BUTTON_W, "Cancel", true),
        ];
        let mut d = KeyDialog {
            ssid: Ssid::default(),
            ap: "/".into(),
            security: Security::Open,
            hidden: true,
            form: Form::new(items),
        };
        d.sync_button();
        d
    }

    pub fn client(&self) -> (i32, i32) {
        if self.hidden { HIDDEN_CLIENT } else { CLIENT }
    }

    /// The network to join: the one picked, or the name typed for a hidden one.
    pub fn ssid(&self) -> Ssid {
        if self.hidden {
            Ssid(self.field("ssid").as_bytes().to_vec())
        } else {
            self.ssid.clone()
        }
    }

    fn security(&self) -> Security {
        if self.hidden {
            let i = self.form.combo_selected("security").unwrap_or(0);
            HIDDEN_KINDS.get(i).copied().unwrap_or_default()
        } else {
            self.security
        }
    }

    pub fn event(&mut self, ev: form::Event) -> Outcome {
        match ev {
            form::Event::Changed(_) => {
                self.sync_button();
                Outcome::Redraw
            }
            form::Event::Pressed("connect") | form::Event::Default => {
                if !self.valid() {
                    Outcome::None
                } else {
                    match self.security() {
                        Security::Open => Outcome::Connect(None),
                        Security::Enterprise => Outcome::Enterprise,
                        _ => Outcome::Connect(Some(Psk::new(self.field("key")))),
                    }
                }
            }
            form::Event::Pressed("cancel") | form::Event::Cancel => Outcome::Cancel,
            form::Event::Redraw => Outcome::Redraw,
            _ => Outcome::None,
        }
    }

    pub fn valid(&self) -> bool {
        if self.hidden && !(1..=SSID_MAX).contains(&self.field("ssid").len()) {
            return false;
        }
        let key = self.field("key");
        match self.security() {
            Security::Open | Security::Enterprise => true,
            security => key == self.field("confirm") && key_ok(key, security),
        }
    }

    pub fn sync_button(&mut self) {
        let valid = self.valid();
        let focused = self.form.focus() == self.form.index_of("connect");
        let security = self.security();
        let keyed = !matches!(security, Security::Open | Security::Enterprise);
        for id in ["key_l", "key", "confirm_l", "confirm"] {
            self.form.set_enabled(id, keyed);
        }
        let word = if security == Security::Enterprise {
            "&Next >"
        } else {
            "&Connect"
        };
        if let Some(Control::Button { enabled, label, .. }) =
            self.form.item_mut("connect").map(|i| &mut i.control)
        {
            *enabled = valid;
            *label = word.into();
        }
        // A greyed button is no tab stop, so the focus cannot stay on it.
        if focused && !valid {
            self.form.focus_edge(false);
        }
    }

    pub fn wipe(&mut self) {
        for id in ["key", "confirm"] {
            if let Some(Control::Edit { edit, .. }) = self.form.item_mut(id).map(|i| &mut i.control)
            {
                edit.wipe();
            }
        }
    }

    fn field(&self, id: &str) -> &str {
        self.form.edit_text(id)
    }

    pub fn window_title() -> String {
        "Wireless Network Connection".into()
    }
}

// Wipes both key fields however the dialog closes.
impl Drop for KeyDialog {
    fn drop(&mut self) {
        self.wipe();
    }
}

pub fn intro_lines(ssid: &str) -> Vec<String> {
    let mut out = reprisekit::text::wrap_chars(
        &format!(
            "The network '{ssid}' requires a network key (also called a WEP key or WPA key). \
             A network key helps prevent unknown intruders from connecting to this network."
        ),
        INTRO_COLS,
    );
    out.push(String::new());
    out.push("Type the key, and then click Connect.".into());
    out
}

fn field(id: &'static str, y: i32) -> Item {
    Item::new(
        id,
        Rect::new(FIELD_X, y, FIELD_W, FIELD_H),
        Control::Edit {
            edit: TextEdit::secret(),
            first: Cell::new(0),
            enabled: true,
        },
    )
}

fn connect_button(enabled: bool) -> Item {
    Item::new(
        "connect",
        Rect::new(CONNECT_X, BTN_Y, BUTTON_W, BUTTON_H),
        Control::Button {
            label: "&Connect".into(),
            enabled,
            default: true,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use reprisekit::classic::widgets::LINE_H;
    use reprisekit::raster::Canvas;

    fn dialog() -> KeyDialog {
        KeyDialog::new(&"Neighbour".into(), "/ap/5", Security::Psk)
    }

    /// The dialog fits its text under every interface and text size.
    #[test]
    fn the_dialog_fits_its_text() {
        let over = reprisekit::interfaces::form_overflows(|_, _, _| {
            vec![("key".into(), dialog().form.clone())]
        });
        assert!(over.is_empty(), "{}", over.join("\n"));
    }

    /// Type `s` into the field called `id`, as the form's own key
    /// handling would.
    fn type_into(d: &mut KeyDialog, id: &str, s: &str) {
        d.form.focus_id(id);
        for ch in s.chars() {
            let ev = d.form.key(form::Key::Char(ch));
            d.event(ev);
        }
    }

    fn rect(d: &KeyDialog, id: &str) -> Rect {
        d.form.item(id).unwrap_or_else(|| panic!("no {id}")).rect
    }

    fn enabled(d: &KeyDialog, id: &str) -> bool {
        match d.form.item(id).unwrap().control {
            Control::Button { enabled, .. } => enabled,
            _ => panic!("{id} is no button"),
        }
    }

    #[test]
    fn key_dialog_validates() {
        let mut d = dialog();
        assert!(!d.valid(), "empty");
        assert!(!enabled(&d, "connect"));
        assert_eq!(d.event(form::Event::Pressed("connect")), Outcome::None);
        type_into(&mut d, "key", "12345678");
        assert!(!d.valid(), "the confirmation is still empty");
        type_into(&mut d, "confirm", "1234567");
        assert!(!d.valid(), "the two do not match");
        assert!(!enabled(&d, "connect"));
        type_into(&mut d, "confirm", "8");
        assert!(d.valid());
        assert!(enabled(&d, "connect"));
        assert_eq!(
            d.event(form::Event::Pressed("connect")),
            Outcome::Connect(Some(Psk::new("12345678")))
        );
        assert_eq!(
            d.event(form::Event::Default),
            Outcome::Connect(Some(Psk::new("12345678")))
        );
        let mut short = dialog();
        type_into(&mut short, "key", "1234567");
        type_into(&mut short, "confirm", "1234567");
        assert!(!short.valid());
        let long = "x".repeat(64);
        let mut over = dialog();
        type_into(&mut over, "key", &long);
        type_into(&mut over, "confirm", &long);
        assert!(!over.valid());
        assert!(!enabled(&over, "connect"));
        let mut edge = dialog();
        let max = "x".repeat(63);
        type_into(&mut edge, "key", &max);
        type_into(&mut edge, "confirm", &max);
        assert!(edge.valid());
        let raw = "0123456789abcdef".repeat(4);
        let mut hex = dialog();
        type_into(&mut hex, "key", &raw);
        type_into(&mut hex, "confirm", &raw);
        assert!(hex.valid());
        assert!(enabled(&hex, "connect"));
        let mut upper = dialog();
        type_into(&mut upper, "key", &raw.to_uppercase());
        type_into(&mut upper, "confirm", &raw.to_uppercase());
        assert!(upper.valid());
    }

    #[test]
    fn the_hidden_dialog_fits_its_text() {
        let over = reprisekit::interfaces::form_overflows(|_, _, _| {
            vec![("hidden".into(), KeyDialog::hidden().form.clone())]
        });
        assert!(over.is_empty(), "{}", over.join("\n"));
    }

    fn pick_security(d: &mut KeyDialog, i: usize) {
        d.form.set_combo("security", i);
        d.event(form::Event::Changed("security"));
    }

    #[test]
    fn a_hidden_network_takes_a_name_and_its_security() {
        let mut d = KeyDialog::hidden();
        assert_eq!(d.ap, "/");
        assert_eq!(d.client(), HIDDEN_CLIENT);
        assert!(!enabled(&d, "connect"), "no name yet");
        assert!(!live(&d, "key"), "an open network takes no key");
        type_into(&mut d, "ssid", "Attic");
        assert_eq!(d.ssid(), "Attic".into());
        assert_eq!(d.event(form::Event::Default), Outcome::Connect(None));
        pick_security(&mut d, 1);
        assert!(live(&d, "key") && live(&d, "confirm"));
        assert!(!enabled(&d, "connect"), "WPA-PSK waits for its key");
        type_into(&mut d, "key", "hunter22");
        type_into(&mut d, "confirm", "hunter22");
        assert_eq!(
            d.event(form::Event::Pressed("connect")),
            Outcome::Connect(Some(Psk::new("hunter22")))
        );
        pick_security(&mut d, 2);
        assert!(!live(&d, "key"));
        assert_eq!(label(&d, "connect"), "&Next >");
        assert_eq!(d.event(form::Event::Default), Outcome::Enterprise);
        let mut long = KeyDialog::hidden();
        type_into(&mut long, "ssid", &"x".repeat(33));
        assert!(!long.valid(), "33 bytes is one past an SSID");
    }

    fn live(d: &KeyDialog, id: &str) -> bool {
        matches!(
            d.form.item(id).unwrap().control,
            Control::Edit { enabled: true, .. }
        )
    }

    fn label(d: &KeyDialog, id: &str) -> String {
        match &d.form.item(id).unwrap().control {
            Control::Button { label, .. } => label.clone(),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_hidden_rows_stack_inside_the_client() {
        let d = KeyDialog::hidden();
        let r = |id: &str| rect(&d, id);
        let rows = ["intro", "ssid", "security", "key", "confirm", "connect"];
        for w in rows.windows(2) {
            assert!(r(w[1]).y > r(w[0]).bottom(), "{} sits below {}", w[1], w[0]);
        }
        assert_eq!(r("cancel").bottom(), HIDDEN_CLIENT.1 - MARGIN);
        assert_eq!(r("connect").y, r("cancel").y);
    }

    #[test]
    fn a_passphrase_is_measured_in_bytes() {
        let wide = "é".repeat(32);
        let mut d = dialog();
        type_into(&mut d, "key", &wide);
        type_into(&mut d, "confirm", &wide);
        assert!(!d.valid(), "64 bytes is one past the limit");
        let short = "éééé";
        let mut d = dialog();
        type_into(&mut d, "key", short);
        type_into(&mut d, "confirm", short);
        assert!(d.valid(), "8 bytes is the minimum");
    }

    #[test]
    fn an_sae_password_takes_any_nonzero_length() {
        let mut d = KeyDialog::new(&"Lab".into(), "/ap/7", Security::Sae);
        assert!(!d.valid(), "an empty password is refused");
        type_into(&mut d, "key", "abc");
        type_into(&mut d, "confirm", "abc");
        assert!(d.valid());
        let long = "x".repeat(100);
        let mut d = KeyDialog::new(&"Lab".into(), "/ap/7", Security::Sae);
        type_into(&mut d, "key", &long);
        type_into(&mut d, "confirm", &long);
        assert!(d.valid());
    }

    #[test]
    fn a_paste_into_both_fields_enables_connect() {
        let mut d = dialog();
        for id in ["key", "confirm"] {
            d.form.focus_id(id);
            let ev = d.form.paste("correct horse\n".into());
            d.event(ev);
        }
        assert!(d.valid());
        assert_eq!(d.field("key"), "correct horse");
        assert!(enabled(&d, "connect"));
    }

    #[test]
    fn wipe_empties_both_key_fields_in_place() {
        let mut d = dialog();
        type_into(&mut d, "key", "hunter22");
        type_into(&mut d, "confirm", "hunter22");
        d.wipe();
        assert_eq!(d.field("key"), "");
        assert_eq!(d.field("confirm"), "");
    }

    #[test]
    fn cancel_and_redraw_pass_through() {
        let mut d = dialog();
        assert_eq!(d.event(form::Event::Pressed("cancel")), Outcome::Cancel);
        assert_eq!(d.event(form::Event::Cancel), Outcome::Cancel);
        assert_eq!(d.event(form::Event::Redraw), Outcome::Redraw);
        assert_eq!(d.event(form::Event::None), Outcome::None);
        assert_eq!(d.event(form::Event::Changed("key")), Outcome::Redraw);
        assert_eq!(d.ssid, "Neighbour".into());
        assert_eq!(d.ap, "/ap/5");
        assert_eq!(KeyDialog::window_title(), "Wireless Network Connection");
    }

    #[test]
    fn the_form_fits_the_client() {
        let d = dialog();
        for id in ["key", "confirm"] {
            assert!(matches!(
                &d.form.item(id).unwrap().control,
                Control::Edit { edit, .. } if edit.is_secret()
            ));
            let r = rect(&d, id);
            assert_eq!(r.right(), CLIENT.0 - MARGIN);
            assert!(r.x > rect(&d, "key_l").right());
        }
        assert!(rect(&d, "confirm").y > rect(&d, "key").bottom());
        assert!(
            rect(&d, "key").y > rect(&d, "intro").bottom(),
            "the fields clear the paragraphs"
        );
        for id in ["connect", "cancel"] {
            let r = rect(&d, id);
            assert!(r.right() <= CLIENT.0 && r.bottom() <= CLIENT.1);
            assert!(r.y > rect(&d, "confirm").bottom());
        }
        assert_eq!(rect(&d, "cancel").right(), CLIENT.0 - MARGIN);
        assert!(rect(&d, "connect").right() < rect(&d, "cancel").x);
        let intro = intro_lines("Neighbour");
        assert!(intro[0].starts_with("The network 'Neighbour' requires"));
        assert!(intro.iter().all(|l| l.chars().count() <= INTRO_COLS));
        assert_eq!(intro[intro.len() - 2], "");
        assert_eq!(
            intro[intro.len() - 1],
            "Type the key, and then click Connect."
        );
        assert_eq!(rect(&d, "intro").height, intro.len() as i32 * LINE_H);
    }

    #[test]
    #[ignore]
    fn render_keydlg() {
        let theme = reprisekit::theme::Theme {
            access_keys: true,
            ..reprisekit::theme::Theme::default()
        };
        let text = reprisekit::text::TextRenderer::load(&reprisekit::theme::FontConfig::default());
        let mut d = dialog();
        type_into(&mut d, "key", "hunter22");
        type_into(&mut d, "confirm", "hunter22");
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
        reprisekit::raster::write_preview(&c, "wireless-key.png");
    }
}
