//! A printer's Properties: General, Sharing, Ports and Device Settings tabs. Apply sends what
//! changed; a new name goes last, as a rename.

use reprisekit::classic::widgets::LINE_H;
use reprisekit::controls::form::{self, Control, Form, Item};
use reprisekit::controls::propsheet::{self, PropSheet};
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::escape_mnemonic;
use reprisekit::textedit::TextEdit;

use crate::cups::{self, Change, Options, Printer, Setting};
use crate::ipp::Value;

pub const CLIENT: (i32, i32) = (380, 440);
const TABS: [&str; 4] = ["General", "Sharing", "Ports", "Device Settings"];
pub const GENERAL: usize = 0;
pub const SHARING: usize = 1;
const PORTS: usize = 2;
const DEVICE: usize = 3;

const NAME: &str = "name";
const LOCATION: &str = "location";
const COMMENT: &str = "comment";
const MODEL: &str = "model";
const FEATURES: &str = "features";
const TEST: &str = "test";
const NOSHARE: &str = "noshare";
const SHARE: &str = "share";
const URI: &str = "uri";
const CONNECTION: &str = "connection";
const MEDIA: &str = "media";
const SIDES: &str = "sides";
const COLOR: &str = "color";
const QUALITY: &str = "quality";
const READING: &str = "reading";

const LABEL_X: i32 = 18;
const VALUE_X: i32 = 100;
const VALUE_W: i32 = CLIENT.0 - VALUE_X - 26;
const FULL_W: i32 = CLIENT.0 - 2 * LABEL_X - 8;

#[derive(Clone, Debug, Default, PartialEq)]
struct Fields {
    name: String,
    location: String,
    comment: String,
    shared: bool,
    uri: String,
    /// Device Settings: the picked keyword of each, `None` until the
    /// printer's choices are read (or when it offers none).
    media: Option<String>,
    sides: Option<String>,
    color: Option<String>,
    quality: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    None,
    Redraw,
    Close,
    TestPage,
    Apply {
        change: Change,
        rename: Option<String>,
        close: bool,
    },
}

pub struct PropsSheet {
    pub printer: String,
    pub sheet: PropSheet,
    applied: Fields,
    pending: Fields,
    options: Option<Options>,
}

fn edit_text(form: &Form, id: &str) -> String {
    match form.item(id).map(|i| &i.control) {
        Some(Control::Edit { edit, .. }) => edit.text.clone(),
        _ => String::new(),
    }
}

fn set_edit(form: &mut Form, id: &str, s: &str) {
    if let Some(Control::Edit { edit, .. }) = form.item_mut(id).map(|i| &mut i.control) {
        *edit = TextEdit::new(s, false);
    }
}

fn features(p: &Printer, o: Option<&Options>) -> Vec<String> {
    let Some(o) = o else {
        return vec!["Reading the printer's features...".into()];
    };
    let yes = |b: bool| if b { "Yes" } else { "No" };
    let color = o.color.choices.iter().any(|c| c == "color");
    let duplex = o.sides.choices.iter().any(|c| c.starts_with("two-sided"));
    let mut l = vec![
        format!("Color: {}", yes(color)),
        format!("Double-sided: {}", yes(duplex)),
    ];
    if let Some(m) = &o.media.default {
        l.push(format!("Paper: {}", cups::media_label(m)));
    }
    if !p.accepting {
        l.push("Not accepting documents".into());
    }
    l
}

pub fn connection(uri: &str) -> &'static str {
    let scheme = uri.split(':').next().unwrap_or_default();
    match scheme {
        "ipp" | "ipps" | "http" | "https" => "Network printer (IPP)",
        "dnssd" | "mdns" => "Network printer (found on the network)",
        "socket" => "Network printer (raw TCP/IP)",
        "lpd" => "Network printer (LPD)",
        "smb" => "Shared printer on another computer (SMB)",
        "usb" => "USB printer",
        "parallel" => "Parallel port printer",
        "serial" => "Serial port printer",
        "cups-pdf" => "Prints to PDF files",
        "file" => "Prints to a file",
        "implicitclass" => "Printer shared by another computer",
        _ => "Other",
    }
}

fn general(icon: Option<Canvas>) -> Form {
    let mut items = vec![
        Item::picture("icon", Rect::new(LABEL_X, 44, 32, 32), icon),
        Item::edit(NAME, Rect::new(70, 50, CLIENT.0 - 70 - 26, 21), "", true),
        Item::new(
            "sep1",
            Rect::new(LABEL_X, 90, FULL_W, 2),
            Control::Separator,
        ),
        Item::label_at("location_label", LABEL_X, 106, 80, "&Location:"),
        Item::edit(LOCATION, Rect::new(VALUE_X, 102, VALUE_W, 21), "", true),
        Item::label_at("comment_label", LABEL_X, 134, 80, "&Comment:"),
        Item::edit(COMMENT, Rect::new(VALUE_X, 130, VALUE_W, 21), "", true),
        Item::label_at("model_label", LABEL_X, 164, 80, "Model:"),
        Item::label_at(MODEL, VALUE_X, 164, VALUE_W, ""),
        Item::group(
            "features_group",
            Rect::new(LABEL_X, 190, FULL_W, 100),
            "Features",
        ),
    ];
    items.push(Item::new(
        FEATURES,
        Rect::new(LABEL_X + 12, 210, FULL_W - 24, LINE_H * 4),
        Control::Lines(Vec::new()),
    ));
    items.push(Item::button_at(
        TEST,
        CLIENT.0 - 26 - 110,
        304,
        110,
        "Print &Test Page",
        true,
    ));
    Form::new(items)
}

fn sharing(icon: Option<Canvas>) -> Form {
    Form::new(vec![
        Item::picture("icon", Rect::new(LABEL_X, 44, 32, 32), icon),
        Item::lines_at(
            "about",
            62,
            46,
            FULL_W - 44,
            &[
                "You can share this printer with other",
                "computers on your network.",
            ],
        ),
        Item::radio(NOSHARE, LABEL_X, 100, "Do &not share this printer", "share"),
        Item::radio(SHARE, LABEL_X, 124, "&Share this printer", "share"),
        Item::lines_at(
            "note",
            LABEL_X,
            156,
            FULL_W,
            &[
                "Other computers see a shared printer only while",
                "CUPS shares printers (cupsctl --share-printers).",
            ],
        ),
    ])
}

fn ports() -> Form {
    Form::new(vec![
        Item::lines_at(
            "about",
            LABEL_X,
            44,
            FULL_W,
            &["Documents print to the printer at this address."],
        ),
        Item::label_at("uri_label", LABEL_X, 76, FULL_W, "&Address (URI):"),
        Item::edit(URI, Rect::new(LABEL_X, 94, FULL_W, 21), "", true),
        Item::label_at("conn_label", LABEL_X, 130, 80, "Connection:"),
        Item::label_at(CONNECTION, VALUE_X, 130, VALUE_W, ""),
        Item::lines_at(
            "examples",
            LABEL_X,
            164,
            FULL_W,
            &[
                "Examples:",
                "  ipp://printer.local/ipp/print",
                "  socket://192.168.1.20:9100",
                "  lpd://server/queue",
            ],
        ),
    ])
}

fn choices(s: &Setting, label: fn(&str) -> String) -> (Vec<String>, usize) {
    let labels: Vec<String> = s.choices.iter().map(|k| label(k)).collect();
    let at = s
        .default
        .as_ref()
        .and_then(|d| s.choices.iter().position(|c| c == d))
        .unwrap_or(0);
    (labels, at)
}

fn device(o: Option<&Options>) -> Form {
    let Some(o) = o else {
        return Form::new(vec![Item::label_at(
            READING,
            LABEL_X,
            48,
            FULL_W,
            "Reading the printer's settings...",
        )]);
    };
    let mut items = vec![Item::lines_at(
        "about",
        LABEL_X,
        44,
        FULL_W,
        &[
            "Documents print with these settings unless they",
            "ask for others.",
        ],
    )];
    let rows: [SettingRow; 4] = [
        (
            "media_label",
            "&Paper size:",
            MEDIA,
            &o.media,
            cups::media_label,
        ),
        (
            "sides_label",
            "T&wo-sided:",
            SIDES,
            &o.sides,
            cups::sides_label,
        ),
        ("color_label", "C&olor:", COLOR, &o.color, cups::color_label),
        (
            "quality_label",
            "&Quality:",
            QUALITY,
            &o.quality,
            cups::quality_label,
        ),
    ];
    for (i, (lid, label, id, setting, words)) in rows.into_iter().enumerate() {
        let y = 92 + i as i32 * 34;
        items.push(Item::label_at(lid, LABEL_X, y + 4, 80, label));
        let (labels, at) = choices(setting, words);
        if labels.is_empty() {
            items.push(
                Item::label_at(id, VALUE_X, y + 4, VALUE_W, "Not offered by this printer")
                    .enabled(false),
            );
            continue;
        }
        let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        let mut combo = Item::combo(id, VALUE_X, y, VALUE_W, &refs, 8);
        if let Control::Combo(c) = &mut combo.control {
            c.selected = at;
        }
        items.push(combo);
    }
    Form::new(items)
}

/// One edit field after a new reading: it takes the new value unless an
/// edit of its own is pending (or always, on the sheet's first reading).
fn sync(first: bool, was: &str, now: &str, pending: &mut String, page: &mut Form, id: &str) {
    if first || (pending == was && was != now) {
        *pending = now.to_string();
        set_edit(page, id, now);
    }
}

type SettingRow<'a> = (
    &'static str,
    &'static str,
    &'static str,
    &'a Setting,
    fn(&str) -> String,
);

fn picked(form: &Form, id: &str, s: &Setting) -> Option<String> {
    let i = form.combo_selected(id)?;
    s.choices.get(i).cloned()
}

fn default_of(s: &Setting) -> Option<String> {
    s.default.clone().or_else(|| s.choices.first().cloned())
}

impl PropsSheet {
    pub fn new(p: &Printer, icon: Option<Canvas>) -> PropsSheet {
        let pages = vec![general(icon.clone()), sharing(icon), ports(), device(None)];
        let mut sheet = PropsSheet {
            printer: p.name.clone(),
            sheet: PropSheet::new(&TABS, pages, CLIENT),
            applied: Fields::default(),
            pending: Fields::default(),
            options: None,
        };
        sheet.refresh(p, true);
        sheet
    }

    pub fn window_title(&self) -> String {
        format!("{} Properties", self.printer)
    }

    pub fn open_tab(&mut self, i: usize) {
        self.sheet.open_tab(i);
    }

    /// A new reading of the printer. Each field that has no edit pending
    /// takes the new value; one being edited keeps the edit.
    pub fn refresh(&mut self, p: &Printer, first: bool) {
        let now = Fields {
            name: p.name.clone(),
            location: p.location.clone(),
            comment: p.info.clone(),
            shared: p.shared,
            uri: p.device_uri.clone(),
            ..self.applied.clone()
        };
        let was = std::mem::replace(&mut self.applied, now);
        let now = &self.applied;
        let pend = &mut self.pending;
        let pages = &mut self.sheet.pages;
        sync(
            first,
            &was.name,
            &now.name,
            &mut pend.name,
            &mut pages[GENERAL],
            NAME,
        );
        sync(
            first,
            &was.location,
            &now.location,
            &mut pend.location,
            &mut pages[GENERAL],
            LOCATION,
        );
        sync(
            first,
            &was.comment,
            &now.comment,
            &mut pend.comment,
            &mut pages[GENERAL],
            COMMENT,
        );
        sync(
            first,
            &was.uri,
            &now.uri,
            &mut pend.uri,
            &mut pages[PORTS],
            URI,
        );
        if first || self.pending.shared == was.shared {
            self.pending.shared = p.shared;
        }
        let page = &mut self.sheet.pages[SHARING];
        page.set_check(SHARE, self.pending.shared);
        page.set_check(NOSHARE, !self.pending.shared);
        self.sheet.pages[GENERAL].set_text(MODEL, &escape_mnemonic(&p.make_model));
        let f = features(p, self.options.as_ref());
        self.sheet.pages[GENERAL].set_lines(FEATURES, &f);
        self.sheet.pages[PORTS].set_text(CONNECTION, &escape_mnemonic(connection(&p.device_uri)));
        let dirty = self.dirty();
        self.sheet.set_apply_enabled(dirty);
    }

    pub fn set_options(&mut self, o: Options, p: &Printer) {
        self.applied.media = default_of(&o.media);
        self.applied.sides = default_of(&o.sides);
        self.applied.color = default_of(&o.color);
        self.applied.quality = default_of(&o.quality);
        self.pending.media = self.applied.media.clone();
        self.pending.sides = self.applied.sides.clone();
        self.pending.color = self.applied.color.clone();
        self.pending.quality = self.applied.quality.clone();
        self.sheet.set_page(DEVICE, device(Some(&o)));
        self.options = Some(o);
        let f = features(p, self.options.as_ref());
        self.sheet.pages[GENERAL].set_lines(FEATURES, &f);
    }

    pub fn options_failed(&mut self, why: &str) {
        let mut form = device(None);
        form.set_text(
            READING,
            &escape_mnemonic(&format!("The settings could not be read. {why}")),
        );
        self.sheet.set_page(DEVICE, form);
    }

    fn dirty(&self) -> bool {
        let mut a = self.applied.clone();
        let mut b = self.pending.clone();
        a.name = a.name.trim().into();
        b.name = b.name.trim().replace(' ', "_");
        a != b
    }

    fn change(&self) -> (Change, Option<String>) {
        let (a, p) = (&self.applied, &self.pending);
        let differs = |x: &String, y: &String| (x != y).then(|| y.clone());
        let mut defaults = Vec::new();
        let settings: [(&str, &Option<String>, &Option<String>, bool); 4] = [
            ("media-default", &a.media, &p.media, false),
            ("sides-default", &a.sides, &p.sides, false),
            ("print-color-mode-default", &a.color, &p.color, false),
            ("print-quality-default", &a.quality, &p.quality, true),
        ];
        for (name, was, now, is_enum) in settings {
            if was != now
                && let Some(v) = now
            {
                let value = if is_enum {
                    Value::Enum(v.parse().unwrap_or(4))
                } else {
                    Value::keyword(v)
                };
                defaults.push((name.to_string(), value));
            }
        }
        let change = Change {
            info: differs(&a.comment, &p.comment),
            location: differs(&a.location, &p.location),
            device_uri: differs(&a.uri, &p.uri).filter(|u| !u.trim().is_empty()),
            shared: (a.shared != p.shared).then_some(p.shared),
            defaults,
        };
        let rename =
            Some(p.name.trim().replace(' ', "_")).filter(|n| !n.is_empty() && *n != a.name);
        (change, rename)
    }

    fn commit(&mut self) -> (Change, Option<String>) {
        let c = self.change();
        let name = self.applied.name.clone();
        self.applied = self.pending.clone();
        // A rename shows once cupsd has the new queue.
        self.applied.name = name;
        c
    }

    fn read(&mut self, id: &str) {
        let pages = &self.sheet.pages;
        let o = self.options.clone().unwrap_or_default();
        match id {
            NAME => self.pending.name = edit_text(&pages[GENERAL], NAME),
            LOCATION => self.pending.location = edit_text(&pages[GENERAL], LOCATION),
            COMMENT => self.pending.comment = edit_text(&pages[GENERAL], COMMENT),
            URI => self.pending.uri = edit_text(&pages[PORTS], URI),
            SHARE | NOSHARE => self.pending.shared = pages[SHARING].checked(SHARE).unwrap_or(false),
            MEDIA => self.pending.media = picked(&pages[DEVICE], MEDIA, &o.media),
            SIDES => self.pending.sides = picked(&pages[DEVICE], SIDES, &o.sides),
            COLOR => self.pending.color = picked(&pages[DEVICE], COLOR, &o.color),
            QUALITY => self.pending.quality = picked(&pages[DEVICE], QUALITY, &o.quality),
            _ => {}
        }
        let dirty = self.dirty();
        self.sheet.set_apply_enabled(dirty);
    }

    pub fn event(&mut self, ev: propsheet::Event) -> Outcome {
        match ev {
            propsheet::Event::None => Outcome::None,
            propsheet::Event::Redraw => Outcome::Redraw,
            propsheet::Event::Page(_, form::Event::Changed(id)) => {
                self.read(id);
                Outcome::Redraw
            }
            propsheet::Event::Page(_, form::Event::Pressed(TEST)) => Outcome::TestPage,
            propsheet::Event::Page(..) => Outcome::Redraw,
            propsheet::Event::Apply if self.dirty() => {
                let (change, rename) = self.commit();
                self.sheet.set_apply_enabled(false);
                Outcome::Apply {
                    change,
                    rename,
                    close: false,
                }
            }
            propsheet::Event::Apply => Outcome::Redraw,
            propsheet::Event::Ok if self.dirty() => {
                let (change, rename) = self.commit();
                Outcome::Apply {
                    change,
                    rename,
                    close: true,
                }
            }
            propsheet::Event::Ok | propsheet::Event::Cancel => Outcome::Close,
        }
    }

    pub fn paste(&mut self, text: String) -> Outcome {
        let ev = self.sheet.paste(text);
        self.event(ev)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn laser() -> Printer {
        Printer {
            name: "Laser".into(),
            info: "Hall".into(),
            location: "2nd floor".into(),
            make_model: "Generic PCL Laser".into(),
            device_uri: "socket://10.0.0.9:9100".into(),
            accepting: true,
            ..Default::default()
        }
    }

    fn options() -> Options {
        Options {
            media: Setting {
                choices: vec!["iso_a4_210x297mm".into(), "na_letter_8.5x11in".into()],
                default: Some("iso_a4_210x297mm".into()),
            },
            sides: Setting {
                choices: vec!["one-sided".into(), "two-sided-long-edge".into()],
                default: Some("one-sided".into()),
            },
            color: Setting::default(),
            quality: Setting {
                choices: vec!["3".into(), "4".into(), "5".into()],
                default: Some("4".into()),
            },
        }
    }

    fn type_into(p: &mut PropsSheet, page: usize, id: &'static str, s: &str) {
        set_edit(&mut p.sheet.pages[page], id, s);
        p.read(id);
    }

    /// Every page fits its text under every interface and text size, Device Settings while
    /// reading, filled in and failed.
    #[test]
    fn every_page_fits_its_text() {
        let over = reprisekit::interfaces::form_overflows(|_, _, _| {
            let mut p = PropsSheet::new(&laser(), None);
            let reading = p.sheet.pages[DEVICE].clone();
            let mut o = options();
            o.color = Setting {
                choices: vec!["monochrome".into(), "color".into()],
                default: Some("color".into()),
            };
            p.set_options(o, &laser());
            let mut failed = PropsSheet::new(&laser(), None);
            failed.options_failed("The printer did not answer.");
            let mut pages: Vec<(String, Form)> = TABS
                .iter()
                .map(|t| t.to_string())
                .zip(p.sheet.pages)
                .collect();
            pages.push(("reading".into(), reading));
            pages.push(("failed".into(), failed.sheet.pages.swap_remove(DEVICE)));
            pages
        });
        assert!(over.is_empty(), "{}", over.join("\n"));
    }

    #[test]
    fn the_fields_open_on_the_printer_and_apply_starts_greyed() {
        let p = PropsSheet::new(&laser(), None);
        assert_eq!(edit_text(&p.sheet.pages[GENERAL], NAME), "Laser");
        assert_eq!(edit_text(&p.sheet.pages[GENERAL], COMMENT), "Hall");
        assert_eq!(
            edit_text(&p.sheet.pages[PORTS], URI),
            "socket://10.0.0.9:9100"
        );
        assert_eq!(p.sheet.pages[SHARING].checked(NOSHARE), Some(true));
        assert!(!p.dirty());
        assert_eq!(p.window_title(), "Laser Properties");
        assert_eq!(connection("socket://x"), "Network printer (raw TCP/IP)");
    }

    #[test]
    fn apply_sends_only_what_changed_and_the_rename_last() {
        let mut p = PropsSheet::new(&laser(), None);
        type_into(&mut p, GENERAL, COMMENT, "Front hall");
        p.sheet.pages[SHARING].set_check(SHARE, true);
        p.read(SHARE);
        type_into(&mut p, GENERAL, NAME, "Hall Laser");
        assert!(p.dirty());
        match p.event(propsheet::Event::Apply) {
            Outcome::Apply {
                change,
                rename,
                close,
            } => {
                assert_eq!(change.info.as_deref(), Some("Front hall"));
                assert_eq!(change.location, None);
                assert_eq!(change.shared, Some(true));
                assert_eq!(change.device_uri, None);
                assert!(change.defaults.is_empty());
                assert_eq!(rename.as_deref(), Some("Hall_Laser"));
                assert!(!close);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn device_settings_fill_in_and_send_their_defaults() {
        let mut p = PropsSheet::new(&laser(), None);
        p.set_options(options(), &laser());
        let page = &p.sheet.pages[DEVICE];
        assert_eq!(page.combo_selected(MEDIA), Some(0));
        assert_eq!(page.combo_selected(QUALITY), Some(1));
        assert!(
            matches!(
                page.item(COLOR).map(|i| &i.control),
                Some(Control::Label { .. })
            ),
            "no colour choices: a greyed line"
        );
        assert!(!p.dirty());
        p.sheet.pages[DEVICE].set_combo(SIDES, 1);
        p.read(SIDES);
        p.sheet.pages[DEVICE].set_combo(QUALITY, 2);
        p.read(QUALITY);
        match p.event(propsheet::Event::Ok) {
            Outcome::Apply { change, close, .. } => {
                assert!(close);
                assert_eq!(
                    change.defaults,
                    [
                        (
                            "sides-default".to_string(),
                            Value::keyword("two-sided-long-edge")
                        ),
                        ("print-quality-default".to_string(), Value::Enum(5)),
                    ]
                );
            }
            other => panic!("{other:?}"),
        }
        let f = features(&laser(), Some(&options()));
        assert_eq!(f[0], "Color: No");
        assert_eq!(f[1], "Double-sided: Yes");
        assert_eq!(f[2], "Paper: A4 (210 x 297 mm)");
    }

    #[test]
    fn a_refresh_keeps_an_edit_and_takes_the_rest() {
        let mut p = PropsSheet::new(&laser(), None);
        type_into(&mut p, GENERAL, COMMENT, "Mine");
        let mut moved = laser();
        moved.info = "Theirs".into();
        moved.location = "Basement".into();
        p.refresh(&moved, false);
        assert_eq!(edit_text(&p.sheet.pages[GENERAL], COMMENT), "Mine");
        assert_eq!(edit_text(&p.sheet.pages[GENERAL], LOCATION), "Basement");
    }

    #[test]
    fn the_test_page_button_says_so() {
        let mut p = PropsSheet::new(&laser(), None);
        assert_eq!(
            p.event(propsheet::Event::Page(GENERAL, form::Event::Pressed(TEST))),
            Outcome::TestPage
        );
        assert_eq!(p.event(propsheet::Event::Cancel), Outcome::Close);
    }

    #[test]
    #[ignore]
    fn render_props() {
        use reprisekit::look::Paint;
        use reprisekit::text::TextRenderer;
        use reprisekit::theme::{FontConfig, Theme};
        let theme = Theme {
            access_keys: true,
            ..Theme::default()
        };
        let text = TextRenderer::load(&FontConfig::default());
        let mut p = PropsSheet::new(&laser(), None);
        p.set_options(options(), &laser());
        for tab in 0..TABS.len() {
            p.open_tab(tab);
            let mut c = Canvas::new(CLIENT.0 as u32, CLIENT.1 as u32);
            c.fill_rect(c.bounds(), theme.colors.button_face);
            let paint = Paint {
                s: 1,
                ..theme.paint()
            };
            propsheet::draw(&mut c, &text, &paint, &p.sheet, true, theme.fonts.size_px);
            reprisekit::raster::write_preview(&c, &format!("printers-props-{tab}.png"));
        }
    }
}
