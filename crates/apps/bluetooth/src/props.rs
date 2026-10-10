//! A paired device's Properties sheet. Apply renames the device and sets
//! whether it is trusted.

use reprisekit::controls::form::{self, Control, Form, Item};
use reprisekit::controls::propsheet::{self, PropSheet};
use reprisekit::raster::{Canvas, Rect};
use reprisekit::textedit::TextEdit;

use crate::bz::{self, Device};

pub const CLIENT: (i32, i32) = (361, 419);
const TABS: [&str; 1] = ["General"];

const ICON: &str = "icon";
const NAME: &str = "name";
const TYPE: &str = "type";
const ADDRESS: &str = "address";
const STATUS: &str = "status";
const BATTERY_LABEL: &str = "battery_label";
const BATTERY: &str = "battery";
const AUTO: &str = "auto";
const CONNECT: &str = "connect";

const LABEL_X: i32 = 18;
const VALUE_X: i32 = 110;
const VALUE_W: i32 = 230;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Settings {
    alias: String,
    trusted: bool,
}

/// What Apply changes; None for what stayed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub alias: Option<String>,
    pub trusted: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    None,
    Redraw,
    Close,
    Toggle,
    Apply(Change),
    ApplyAndClose(Change),
}

pub struct PropsSheet {
    pub path: String,
    pub sheet: PropSheet,
    applied: Settings,
    pending: Settings,
}

pub(crate) fn edit_text(form: &Form, id: &str) -> String {
    match form.item(id).map(|i| &i.control) {
        Some(Control::Edit { edit, .. }) => edit.text.clone(),
        _ => String::new(),
    }
}

pub(crate) fn set_edit(form: &mut Form, id: &str, s: &str) {
    if let Some(Control::Edit { edit, .. }) = form.item_mut(id).map(|i| &mut i.control) {
        *edit = TextEdit::new(s, false);
    }
}

fn page(icon: Canvas) -> Form {
    Form::new(vec![
        Item::picture(ICON, Rect::new(18, 44, 32, 32), Some(icon)),
        Item::edit(NAME, Rect::new(70, 50, 272, 21), "", true),
        Item::new("sep1", Rect::new(18, 90, 324, 2), Control::Separator),
        Item::label_at("type_label", LABEL_X, 102, 90, "Type:"),
        Item::label_at(TYPE, VALUE_X, 102, VALUE_W, ""),
        Item::label_at("address_label", LABEL_X, 124, 90, "Address:"),
        Item::label_at(ADDRESS, VALUE_X, 124, VALUE_W, ""),
        Item::label_at("status_label", LABEL_X, 146, 90, "Status:"),
        Item::label_at(STATUS, VALUE_X, 146, VALUE_W, ""),
        Item::label_at(BATTERY_LABEL, LABEL_X, 168, 90, "Battery:"),
        Item::label_at(BATTERY, VALUE_X, 168, VALUE_W, ""),
        Item::new("sep2", Rect::new(18, 194, 324, 2), Control::Separator),
        Item::check(AUTO, 18, 208, "Connect to this device auto&matically"),
        Item::button_at(CONNECT, 18, 236, 88, "&Connect", true),
    ])
}

impl PropsSheet {
    pub fn new(d: &Device, icon: Canvas) -> PropsSheet {
        let s = Settings {
            alias: d.alias.clone(),
            trusted: d.trusted,
        };
        let mut p = PropsSheet {
            path: d.path.clone(),
            sheet: PropSheet::new(&TABS, vec![page(icon)], CLIENT),
            applied: s.clone(),
            pending: s,
        };
        set_edit(&mut p.sheet.pages[0], NAME, &d.alias);
        p.refresh(d);
        p
    }

    pub fn window_title(&self) -> String {
        format!("{} Properties", self.applied.alias)
    }

    /// Whether the name field holds an unapplied edit (whitespace-only changes do not count).
    fn name_dirty(&self) -> bool {
        self.pending.alias.trim() != self.applied.alias
    }

    fn trusted_dirty(&self) -> bool {
        self.pending.trusted != self.applied.trusted
    }

    fn dirty(&self) -> bool {
        self.name_dirty() || self.trusted_dirty()
    }

    /// Takes a new device snapshot. Each field resyncs on its own: one with an
    /// unapplied edit stays. The name field is rewritten only when the name
    /// changed, so its caret and text survive other updates.
    pub fn refresh(&mut self, d: &Device) {
        let name_dirty = self.name_dirty();
        let trusted_dirty = self.trusted_dirty();
        let renamed = d.alias != self.applied.alias;
        self.applied = Settings {
            alias: d.alias.clone(),
            trusted: d.trusted,
        };
        let page = &mut self.sheet.pages[0];
        if renamed && !name_dirty {
            self.pending.alias = self.applied.alias.clone();
            set_edit(page, NAME, &d.alias);
        }
        if !trusted_dirty {
            self.pending.trusted = self.applied.trusted;
        }
        page.set_text(TYPE, d.kind.name());
        page.set_text(ADDRESS, &d.address);
        page.set_text(
            STATUS,
            &bz::status_line(&Device {
                battery: None,
                ..d.clone()
            }),
        );
        match d.battery {
            Some(b) => {
                page.set_text(BATTERY_LABEL, "Battery:");
                page.set_text(BATTERY, &format!("{b}%"));
            }
            None => {
                page.set_text(BATTERY_LABEL, "");
                page.set_text(BATTERY, "");
            }
        }
        page.set_check(AUTO, self.pending.trusted);
        if let Some(Item {
            control: Control::Button { label, .. },
            ..
        }) = page.item_mut(CONNECT)
        {
            *label = if d.connected {
                "&Disconnect"
            } else {
                "&Connect"
            }
            .into();
        }
        let dirty = self.dirty();
        self.sheet.set_apply_enabled(dirty);
    }

    fn change(&self) -> Change {
        Change {
            alias: self
                .name_dirty()
                .then(|| self.pending.alias.trim().to_string()),
            trusted: self.trusted_dirty().then_some(self.pending.trusted),
        }
    }

    /// The change to send, with `applied` caught up to it (name trimmed, so a trailing space is not pending forever).
    fn commit(&mut self) -> Change {
        let c = self.change();
        self.applied.alias = self.pending.alias.trim().to_string();
        self.applied.trusted = self.pending.trusted;
        c
    }

    fn read(&mut self, id: &str) {
        let page = &self.sheet.pages[0];
        match id {
            NAME => self.pending.alias = edit_text(page, NAME),
            AUTO => self.pending.trusted = page.checked(AUTO).unwrap_or(false),
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
            propsheet::Event::Page(_, form::Event::Pressed(CONNECT)) => Outcome::Toggle,
            propsheet::Event::Page(..) => Outcome::Redraw,
            propsheet::Event::Apply if self.dirty() => {
                let c = self.commit();
                self.sheet.set_apply_enabled(false);
                Outcome::Apply(c)
            }
            propsheet::Event::Apply => Outcome::Redraw,
            propsheet::Event::Ok if self.dirty() => {
                let c = self.commit();
                Outcome::ApplyAndClose(c)
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
    use crate::bz::Kind;
    use reprisekit::controls::form::Control;

    fn cans() -> Device {
        Device {
            path: "/d/1".into(),
            address: "AA:BB:CC:DD:EE:01".into(),
            alias: "Cans".into(),
            kind: Kind::Headphones,
            paired: true,
            trusted: true,
            connected: true,
            battery: Some(80),
            ..Device::default()
        }
    }

    fn label(p: &PropsSheet, id: &str) -> String {
        match p.sheet.pages[0].item(id).map(|i| &i.control) {
            Some(Control::Label { text, .. }) => text.clone(),
            Some(Control::Button { label, .. }) => label.clone(),
            _ => String::new(),
        }
    }

    fn sheet() -> PropsSheet {
        PropsSheet::new(&cans(), Canvas::new(32, 32))
    }

    #[test]
    fn the_rows_show_type_address_status_and_battery() {
        let p = sheet();
        assert_eq!(label(&p, TYPE), "Headphones");
        assert_eq!(label(&p, ADDRESS), "AA:BB:CC:DD:EE:01");
        assert_eq!(label(&p, STATUS), "Connected");
        assert_eq!(label(&p, BATTERY), "80%");
        assert_eq!(label(&p, CONNECT), "&Disconnect");
        assert_eq!(p.sheet.pages[0].checked(AUTO), Some(true));
        assert_eq!(p.window_title(), "Cans Properties");
    }

    #[test]
    fn a_missing_battery_hides_its_row() {
        let mut p = sheet();
        p.refresh(&Device {
            battery: None,
            ..cans()
        });
        assert_eq!(label(&p, BATTERY_LABEL), "");
        assert_eq!(label(&p, BATTERY), "");
    }

    #[test]
    fn a_new_name_and_the_check_apply_together() {
        let mut p = sheet();
        set_edit(&mut p.sheet.pages[0], NAME, "Buds");
        assert_eq!(
            p.event(propsheet::Event::Page(0, form::Event::Changed(NAME))),
            Outcome::Redraw
        );
        p.sheet.pages[0].set_check(AUTO, false);
        p.event(propsheet::Event::Page(0, form::Event::Changed(AUTO)));
        assert_eq!(
            p.event(propsheet::Event::Ok),
            Outcome::ApplyAndClose(Change {
                alias: Some("Buds".into()),
                trusted: Some(false)
            })
        );
    }

    #[test]
    fn connect_toggles_and_its_label_follows_the_device() {
        let mut p = sheet();
        assert_eq!(
            p.event(propsheet::Event::Page(0, form::Event::Pressed(CONNECT))),
            Outcome::Toggle
        );
        p.refresh(&Device {
            connected: false,
            ..cans()
        });
        assert_eq!(label(&p, CONNECT), "&Connect");
        assert_eq!(label(&p, STATUS), "Not connected");
    }

    #[test]
    fn a_refresh_keeps_a_name_being_typed() {
        let mut p = sheet();
        set_edit(&mut p.sheet.pages[0], NAME, "Bu");
        p.event(propsheet::Event::Page(0, form::Event::Changed(NAME)));
        p.refresh(&cans());
        assert_eq!(edit_text(&p.sheet.pages[0], NAME), "Bu");
    }

    #[test]
    fn a_rename_elsewhere_does_not_undo_an_unticked_auto_connect() {
        let mut p = sheet();
        p.sheet.pages[0].set_check(AUTO, false);
        p.event(propsheet::Event::Page(0, form::Event::Changed(AUTO)));
        p.refresh(&Device {
            alias: "Buds".into(),
            ..cans()
        });
        assert_eq!(
            p.event(propsheet::Event::Ok),
            Outcome::ApplyAndClose(Change {
                alias: None,
                trusted: Some(false)
            })
        );
        assert_eq!(edit_text(&p.sheet.pages[0], NAME), "Buds");
    }

    #[test]
    fn a_snapshot_with_the_same_name_keeps_the_field() {
        let mut p = sheet();
        set_edit(&mut p.sheet.pages[0], NAME, "Cans ");
        p.event(propsheet::Event::Page(0, form::Event::Changed(NAME)));
        p.refresh(&Device {
            battery: Some(70),
            ..cans()
        });
        assert_eq!(edit_text(&p.sheet.pages[0], NAME), "Cans ");
    }

    #[test]
    fn alt_a_presses_apply_and_leaves_auto_connect_unchecked() {
        let mut p = sheet();
        set_edit(&mut p.sheet.pages[0], NAME, "Buds");
        p.event(propsheet::Event::Page(0, form::Event::Changed(NAME)));
        let before = p.sheet.pages[0].checked(AUTO);
        assert_eq!(
            p.sheet.key(form::Key::Access('a'), None),
            propsheet::Event::Apply
        );
        assert_eq!(p.sheet.pages[0].checked(AUTO), before);
    }

    #[test]
    fn a_whitespace_padded_name_leaves_apply_greyed() {
        let mut p = sheet();
        set_edit(&mut p.sheet.pages[0], NAME, "Cans ");
        p.event(propsheet::Event::Page(0, form::Event::Changed(NAME)));
        match p.sheet.pages[0].item(propsheet::APPLY).map(|i| &i.control) {
            Some(Control::Button { enabled, .. }) => assert!(!enabled),
            _ => panic!("no apply button"),
        }
    }
}
