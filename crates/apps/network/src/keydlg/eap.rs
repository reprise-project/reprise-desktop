//! The login an 802.1X ("WPA Enterprise") network asks for: the EAP method, its certificates and
//! the user's credentials, laid out on one page with the rows a method does not use greyed.

use std::cell::Cell;
use std::path::{Path, PathBuf};

use reprisekit::classic::widgets::{BUTTON_GAP, BUTTON_H, BUTTON_W, LINE_H};
use reprisekit::controls::combo::ListItem;
use reprisekit::controls::form::{self, Control, Form, Item};
use reprisekit::raster::Rect;
use reprisekit::textedit::TextEdit;

use crate::nm::{Eap, Enterprise, Inner, Psk, Ssid};

pub const CLIENT: (i32, i32) = (420, 408);

const MARGIN: i32 = 12;
const TEXT_W: i32 = CLIENT.0 - 2 * MARGIN;
const TEXT_COLS: usize = 66;
const LABEL_W: i32 = 132;
const FIELD_X: i32 = 150;
const FIELD_W: i32 = CLIENT.0 - MARGIN - FIELD_X;
const FIELD_H: i32 = 20;
const BROWSE_W: i32 = BUTTON_W;
const PATH_W: i32 = FIELD_W - BUTTON_GAP - BROWSE_W;
const LABEL_DY: i32 = 4;
const PITCH: i32 = 28;

const METHOD_Y: i32 = 60;
const ANON_Y: i32 = METHOD_Y + PITCH;
const CA_Y: i32 = ANON_Y + PITCH;
const NO_CA_Y: i32 = CA_Y + PITCH + 2;
const WARNING_Y: i32 = NO_CA_Y + 18;
const INNER_Y: i32 = WARNING_Y + 2 * LINE_H + 6;
const USER_Y: i32 = INNER_Y + PITCH;
const PASSWORD_Y: i32 = USER_Y + PITCH;
const CERT_Y: i32 = PASSWORD_Y + PITCH;
const PKEY_Y: i32 = CERT_Y + PITCH;
const PKEY_PW_Y: i32 = PKEY_Y + PITCH;

const BTN_Y: i32 = CLIENT.1 - MARGIN - BUTTON_H;
const CANCEL_X: i32 = CLIENT.0 - MARGIN - BUTTON_W;
const CONNECT_X: i32 = CANCEL_X - BUTTON_GAP - BUTTON_W;

const NO_CA_WARNING: &str = "Not using a Certificate Authority (CA) certificate can result in \
                             connections to insecure, rogue Wi-Fi networks.";

/// The path fields, each with its Browse... button `<id>_browse`.
pub const PATHS: [&str; 3] = ["ca", "cert", "pkey"];
const SECRETS: [&str; 2] = ["password", "pkey_pw"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    None,
    Redraw,
    /// Pick a file for path field `.0`.
    Browse(&'static str),
    Connect(Box<Enterprise>),
    Cancel,
}

pub struct EapDialog {
    pub ssid: Ssid,
    pub ap: String,
    /// The network is in no scan: its profile probes for it by name.
    pub hidden: bool,
    pub form: Form,
}

impl EapDialog {
    pub fn new(ssid: &Ssid, ap: &str) -> EapDialog {
        let intro = intro_lines(&ssid.to_string());
        let intro: Vec<&str> = intro.iter().map(String::as_str).collect();
        let methods: Vec<&str> = Eap::ALL.iter().map(|m| m.label()).collect();
        let label = |id, y, text: &str| Item::label_at(id, MARGIN, y + LABEL_DY, LABEL_W, text);
        let items = vec![
            Item::lines_at("intro", MARGIN, MARGIN, TEXT_W, &intro),
            label("method_l", METHOD_Y, "&Authentication:"),
            Item::combo(
                "method",
                FIELD_X,
                METHOD_Y,
                FIELD_W,
                &methods,
                methods.len(),
            ),
            label("anon_l", ANON_Y, "A&nonymous identity:"),
            Item::edit("anon", field_r(ANON_Y, FIELD_W), "", true),
            label("ca_l", CA_Y, "CA cer&tificate:"),
            Item::edit("ca", field_r(CA_Y, PATH_W), "", true),
            browse("ca_browse", CA_Y),
            Item::check("no_ca", FIELD_X, NO_CA_Y, "No CA certificate is re&quired"),
            Item::lines(
                "warning",
                Rect::new(MARGIN, WARNING_Y, TEXT_W, 2 * LINE_H),
                &[],
            ),
            label("inner_l", INNER_Y, "&Inner authentication:"),
            Item::combo("inner", FIELD_X, INNER_Y, FIELD_W, &[], 3),
            label("user_l", USER_Y, "&Username:"),
            Item::edit("user", field_r(USER_Y, FIELD_W), "", true),
            label("password_l", PASSWORD_Y, "&Password:"),
            secret("password", PASSWORD_Y),
            label("cert_l", CERT_Y, "Us&er certificate:"),
            Item::edit("cert", field_r(CERT_Y, PATH_W), "", true),
            browse("cert_browse", CERT_Y),
            label("pkey_l", PKEY_Y, "Private &key:"),
            Item::edit("pkey", field_r(PKEY_Y, PATH_W), "", true),
            browse("pkey_browse", PKEY_Y),
            label("pkey_pw_l", PKEY_PW_Y, "Private key pass&word:"),
            secret("pkey_pw", PKEY_PW_Y),
            Item::button(
                "connect",
                Rect::new(CONNECT_X, BTN_Y, BUTTON_W, BUTTON_H),
                "&Connect",
                false,
                true,
            ),
            Item::button_at("cancel", CANCEL_X, BTN_Y, BUTTON_W, "Cancel", true),
        ];
        let mut d = EapDialog {
            ssid: ssid.clone(),
            ap: ap.into(),
            hidden: false,
            form: Form::new(items),
        };
        d.sync_method();
        d
    }

    pub fn window_title() -> String {
        "Wireless Network Connection".into()
    }

    pub fn event(&mut self, ev: form::Event) -> Outcome {
        match ev {
            form::Event::Changed("method") => {
                self.sync_method();
                Outcome::Redraw
            }
            form::Event::Changed(_) => {
                self.sync();
                Outcome::Redraw
            }
            form::Event::Pressed("connect") | form::Event::Default => match self.login() {
                Some(e) => Outcome::Connect(Box::new(e)),
                None => Outcome::None,
            },
            form::Event::Pressed("cancel") | form::Event::Cancel => Outcome::Cancel,
            form::Event::Pressed(id) => PATHS
                .into_iter()
                .find(|p| id.strip_suffix("_browse") == Some(p))
                .map_or(Outcome::None, Outcome::Browse),
            form::Event::Redraw => Outcome::Redraw,
            _ => Outcome::None,
        }
    }

    pub fn method(&self) -> Eap {
        let i = self.form.combo_selected("method").unwrap_or(0);
        Eap::ALL.get(i).copied().unwrap_or_default()
    }

    fn inner(&self) -> Inner {
        let i = self.form.combo_selected("inner").unwrap_or(0);
        self.method().inner().get(i).copied().unwrap_or_default()
    }

    /// The path in field `id`, if one was typed or picked.
    pub fn path_of(&self, id: &str) -> Option<PathBuf> {
        let t = self.form.edit_text(id);
        (!t.is_empty()).then(|| PathBuf::from(t))
    }

    /// Put `path` in path field `id`, as Browse... does.
    pub fn set_path(&mut self, id: &str, path: &Path) {
        if let Some(Control::Edit { edit, .. }) = self.form.item_mut(id).map(|i| &mut i.control) {
            *edit = TextEdit::new(path.to_string_lossy(), false);
        }
        self.sync();
    }

    /// The method's own inner methods in the Inner authentication list, keeping the pick when
    /// the new method offers it too; then the rows it greys.
    fn sync_method(&mut self) {
        let was = self
            .form
            .item("inner")
            .and_then(|i| match &i.control {
                Control::Combo(c) => c.items.get(c.selected).map(|it| it.text.clone()),
                _ => None,
            })
            .unwrap_or_default();
        let choices = self.method().inner();
        if let Some(Control::Combo(c)) = self.form.item_mut("inner").map(|i| &mut i.control) {
            c.items = choices.iter().map(|m| ListItem::new(m.label())).collect();
            c.selected = choices.iter().position(|m| m.label() == was).unwrap_or(0);
        }
        self.sync();
    }

    /// Grey what the method and the CA check box leave unused, and the Connect button until
    /// the login is complete.
    fn sync(&mut self) {
        let tls = self.method() == Eap::Tls;
        let no_ca = self.form.checked("no_ca") == Some(true);
        for id in [
            "anon_l",
            "anon",
            "inner_l",
            "inner",
            "password_l",
            "password",
        ] {
            self.form.set_enabled(id, !tls);
        }
        for id in [
            "cert_l",
            "cert",
            "cert_browse",
            "pkey_l",
            "pkey",
            "pkey_browse",
        ] {
            self.form.set_enabled(id, tls);
        }
        for id in ["pkey_pw_l", "pkey_pw"] {
            self.form.set_enabled(id, tls);
        }
        for id in ["ca_l", "ca", "ca_browse"] {
            self.form.set_enabled(id, !no_ca);
        }
        let warning = if no_ca {
            reprisekit::text::wrap_chars(NO_CA_WARNING, TEXT_COLS)
        } else {
            Vec::new()
        };
        self.form.set_lines("warning", &warning);
        let ok = self.login().is_some();
        let focused = self.form.focus() == self.form.index_of("connect");
        self.form.set_enabled("connect", ok);
        // A greyed button is no tab stop, so the focus cannot stay on it.
        if focused && !ok {
            self.form.focus_edge(false);
        }
    }

    /// What the form says, once it says enough to log in: a username, a CA certificate or
    /// none on purpose, and a password (PEAP, TTLS) or a certificate and key (TLS). Paths
    /// must be absolute, as NetworkManager keeps them.
    pub fn login(&self) -> Option<Enterprise> {
        let eap = self.method();
        let identity = self.form.edit_text("user");
        if identity.is_empty() {
            return None;
        }
        let ca_cert = if self.form.checked("no_ca") == Some(true) {
            None
        } else {
            Some(self.absolute("ca")?)
        };
        let mut e = Enterprise {
            eap,
            ca_cert,
            identity: identity.into(),
            ..Enterprise::default()
        };
        match eap {
            Eap::Peap | Eap::Ttls => {
                let password = self.form.edit_text("password");
                if password.is_empty() {
                    return None;
                }
                e.anonymous_identity = self.form.edit_text("anon").into();
                e.inner = self.inner();
                e.password = Psk::new(password);
            }
            Eap::Tls => {
                e.client_cert = Some(self.absolute("cert")?);
                e.private_key = Some(self.absolute("pkey")?);
                e.private_key_password = Psk::new(self.form.edit_text("pkey_pw"));
            }
        }
        Some(e)
    }

    fn absolute(&self, id: &str) -> Option<PathBuf> {
        self.path_of(id).filter(|p| p.is_absolute())
    }

    pub fn wipe(&mut self) {
        for id in SECRETS {
            if let Some(Control::Edit { edit, .. }) = self.form.item_mut(id).map(|i| &mut i.control)
            {
                edit.wipe();
            }
        }
    }
}

// Wipes both password fields however the dialog closes.
impl Drop for EapDialog {
    fn drop(&mut self) {
        self.wipe();
    }
}

fn intro_lines(ssid: &str) -> Vec<String> {
    reprisekit::text::wrap_chars(
        &format!(
            "The network '{ssid}' asks for an 802.1X login. Type the details your network \
             administrator gave you, and then click Connect."
        ),
        TEXT_COLS,
    )
}

fn field_r(y: i32, w: i32) -> Rect {
    Rect::new(FIELD_X, y, w, FIELD_H)
}

fn browse(id: &'static str, y: i32) -> Item {
    Item::button_at(
        id,
        CLIENT.0 - MARGIN - BROWSE_W,
        y - 2,
        BROWSE_W,
        "Browse...",
        true,
    )
}

fn secret(id: &'static str, y: i32) -> Item {
    Item::new(
        id,
        field_r(y, FIELD_W),
        Control::Edit {
            edit: TextEdit::secret(),
            first: Cell::new(0),
            enabled: true,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use reprisekit::raster::Canvas;

    fn dialog() -> EapDialog {
        EapDialog::new(&"eduroam".into(), "/ap/8")
    }

    fn type_into(d: &mut EapDialog, id: &str, s: &str) {
        d.form.focus_id(id);
        for ch in s.chars() {
            let ev = d.form.key(form::Key::Char(ch));
            d.event(ev);
        }
    }

    fn pick(d: &mut EapDialog, id: &'static str, i: usize) {
        d.form.set_combo(id, i);
        d.event(form::Event::Changed(id));
    }

    fn tick_no_ca(d: &mut EapDialog) {
        d.form.set_check("no_ca", true);
        d.event(form::Event::Changed("no_ca"));
    }

    fn live(d: &EapDialog, id: &str) -> bool {
        match &d.form.item(id).unwrap().control {
            Control::Button { enabled, .. }
            | Control::Edit { enabled, .. }
            | Control::Label { enabled, .. } => *enabled,
            Control::Combo(c) => c.enabled,
            other => panic!("{id} is {other:?}"),
        }
    }

    fn inner_labels(d: &EapDialog) -> Vec<String> {
        match &d.form.item("inner").unwrap().control {
            Control::Combo(c) => c.items.iter().map(|i| i.text.clone()).collect(),
            other => panic!("{other:?}"),
        }
    }

    /// The dialog fits its text under every interface and text size, with the CA warning shown.
    #[test]
    fn the_dialog_fits_its_text() {
        let over = reprisekit::interfaces::form_overflows(|_, _, _| {
            let mut d = dialog();
            tick_no_ca(&mut d);
            pick(&mut d, "method", 1);
            vec![("eap".into(), d.form.clone())]
        });
        assert!(over.is_empty(), "{}", over.join("\n"));
    }

    #[test]
    fn a_peap_login_needs_a_ca_a_username_and_a_password() {
        let mut d = dialog();
        assert_eq!(d.method(), Eap::Peap);
        assert!(!live(&d, "connect"));
        type_into(&mut d, "user", "alice@uni.example");
        type_into(&mut d, "password", "hunter22");
        assert!(!live(&d, "connect"), "no CA certificate yet");
        d.set_path("ca", Path::new("certs/uni.pem"));
        assert!(!live(&d, "connect"), "a relative path is refused");
        d.set_path("ca", Path::new("/etc/ssl/certs/uni.pem"));
        assert!(live(&d, "connect"));
        type_into(&mut d, "anon", "anonymous@uni.example");
        let Outcome::Connect(e) = d.event(form::Event::Default) else {
            panic!("no login");
        };
        assert_eq!(
            *e,
            Enterprise {
                eap: Eap::Peap,
                anonymous_identity: "anonymous@uni.example".into(),
                ca_cert: Some("/etc/ssl/certs/uni.pem".into()),
                inner: Inner::Mschapv2,
                identity: "alice@uni.example".into(),
                password: Psk::new("hunter22"),
                ..Enterprise::default()
            }
        );
    }

    #[test]
    fn no_ca_greys_the_ca_row_and_shows_the_warning() {
        let mut d = dialog();
        assert!(live(&d, "ca") && live(&d, "ca_browse"));
        assert_eq!(
            d.form.item("warning").unwrap().control,
            Control::Lines(vec![])
        );
        tick_no_ca(&mut d);
        assert!(!live(&d, "ca") && !live(&d, "ca_browse") && !live(&d, "ca_l"));
        let Control::Lines(lines) = &d.form.item("warning").unwrap().control else {
            panic!("the warning is a note");
        };
        assert!(
            lines
                .join(" ")
                .starts_with("Not using a Certificate Authority (CA)")
        );
        type_into(&mut d, "user", "bob");
        type_into(&mut d, "password", "hunter22");
        assert!(matches!(
            d.event(form::Event::Pressed("connect")),
            Outcome::Connect(e) if e.ca_cert.is_none()
        ));
    }

    #[test]
    fn each_method_offers_its_own_inner_methods() {
        let mut d = dialog();
        assert_eq!(inner_labels(&d), ["MSCHAPv2", "GTC"]);
        pick(&mut d, "inner", 1);
        pick(&mut d, "method", 1);
        assert_eq!(d.method(), Eap::Ttls);
        assert_eq!(inner_labels(&d), ["MSCHAPv2", "PAP", "GTC"]);
        assert_eq!(d.inner(), Inner::Gtc, "the pick survives the switch");
        pick(&mut d, "inner", 1);
        assert_eq!(d.inner(), Inner::Pap);
        pick(&mut d, "method", 0);
        assert_eq!(d.inner(), Inner::Mschapv2, "PEAP has no PAP");
    }

    #[test]
    fn tls_trades_the_password_for_a_certificate_and_key() {
        let mut d = dialog();
        assert!(!live(&d, "cert") && !live(&d, "pkey") && !live(&d, "pkey_pw"));
        pick(&mut d, "method", 2);
        for id in ["inner", "password", "anon"] {
            assert!(!live(&d, id), "{id} greys under TLS");
        }
        for id in ["cert", "cert_browse", "pkey", "pkey_browse", "pkey_pw"] {
            assert!(live(&d, id), "{id} is live under TLS");
        }
        tick_no_ca(&mut d);
        type_into(&mut d, "user", "alice");
        d.set_path("cert", Path::new("/home/alice/alice.crt"));
        assert!(!live(&d, "connect"), "no private key yet");
        d.set_path("pkey", Path::new("/home/alice/alice.key"));
        assert!(live(&d, "connect"), "a key without a password is fine");
        type_into(&mut d, "pkey_pw", "swordfish");
        let Outcome::Connect(e) = d.event(form::Event::Default) else {
            panic!("no login");
        };
        assert_eq!(e.eap, Eap::Tls);
        assert_eq!(e.client_cert, Some("/home/alice/alice.crt".into()));
        assert_eq!(e.private_key, Some("/home/alice/alice.key".into()));
        assert_eq!(&*e.private_key_password, "swordfish");
        assert!(e.password.is_empty());
    }

    #[test]
    fn browse_asks_for_its_own_field() {
        let mut d = dialog();
        assert_eq!(
            d.event(form::Event::Pressed("ca_browse")),
            Outcome::Browse("ca")
        );
        assert_eq!(
            d.event(form::Event::Pressed("pkey_browse")),
            Outcome::Browse("pkey")
        );
        assert_eq!(d.event(form::Event::Pressed("cancel")), Outcome::Cancel);
        assert_eq!(d.event(form::Event::Cancel), Outcome::Cancel);
    }

    #[test]
    fn the_secrets_are_secret_fields_and_wipe_in_place() {
        let mut d = dialog();
        pick(&mut d, "method", 2);
        type_into(&mut d, "pkey_pw", "swordfish");
        pick(&mut d, "method", 0);
        type_into(&mut d, "password", "hunter22");
        for id in SECRETS {
            assert!(matches!(
                &d.form.item(id).unwrap().control,
                Control::Edit { edit, .. } if edit.is_secret()
            ));
        }
        d.wipe();
        assert_eq!(d.form.edit_text("password"), "");
        assert_eq!(d.form.edit_text("pkey_pw"), "");
    }

    #[test]
    fn the_rows_stack_inside_the_client() {
        let d = dialog();
        let r = |id: &str| d.form.item(id).unwrap().rect;
        let rows = [
            "method", "anon", "ca", "no_ca", "warning", "inner", "user", "password", "cert",
            "pkey", "pkey_pw", "connect",
        ];
        assert!(r("method").y > r("intro").y + intro_lines(&"x".repeat(32)).len() as i32 * LINE_H);
        for w in rows.windows(2) {
            assert!(r(w[1]).y > r(w[0]).y, "{} sits below {}", w[1], w[0]);
        }
        for id in PATHS {
            let b = r(&format!("{id}_browse"));
            assert!(r(id).right() < b.x, "{id}'s field ends before its button");
            assert_eq!(b.right(), CLIENT.0 - MARGIN);
        }
        assert_eq!(r("cancel").right(), CLIENT.0 - MARGIN);
        assert!(r("cancel").bottom() <= CLIENT.1);
        assert!(r("connect").y > r("pkey_pw").bottom());
    }

    #[test]
    #[ignore]
    fn render_eapdlg() {
        let theme = reprisekit::theme::Theme {
            access_keys: true,
            ..reprisekit::theme::Theme::default()
        };
        let text = reprisekit::text::TextRenderer::load(&reprisekit::theme::FontConfig::default());
        let mut d = dialog();
        tick_no_ca(&mut d);
        type_into(&mut d, "user", "alice");
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
        reprisekit::raster::write_preview(&c, "wireless-eap.png");
    }
}
