//! The Add Printer Wizard. Nothing here talks to cupsd: the host feeds it the devices and
//! drivers it reads and turns the [`Outcome`]s into commands.

use std::path::PathBuf;

use reprisekit::access::{ROOT, Tree};
use reprisekit::classic::widgets::BUTTON_H;
use reprisekit::controls::form::{self, Control, Form, Item};
use reprisekit::controls::listbox::ListBox;
use reprisekit::controls::wizard::{self, INTERIOR_BODY, Style, Wizard};
use reprisekit::look::Paint;
use reprisekit::raster::{Canvas, Rect};

use crate::cups::{self, Device, Driver, NewPrinter, Ppd, Snapshot};
use crate::folder::check_name;

const FOUND: &str = "found";
const DEVICES: &str = "devices";
const STATUS: &str = "status";
const AGAIN: &str = "again";
const TYPED: &str = "typed";
const ADDRESS: &str = "address";
const SEARCH: &str = "search";
const DRIVERS: &str = "drivers";
const DISK: &str = "disk";
const NAME: &str = "name";
const LOCATION: &str = "location";
const DEFAULT: &str = "default";
const TEST: &str = "test";
const ERROR: &str = "error";

const MAX_DRIVERS: usize = 400;

const B: Rect = INTERIOR_BODY;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Choose,
    Driver,
    Name,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    None,
    Redraw,
    /// Ask cupsd's backends again; `prompt` lets cups-pk-helper ask for the admin password.
    Search {
        prompt: bool,
    },
    Drivers,
    HaveDisk,
    Add {
        printer: NewPrinter,
        default: bool,
        test_page: bool,
    },
    Close,
}

/// What avahi-daemon has said so far.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Network {
    Browsing,
    Found(Vec<Device>),
    Off,
}

pub struct AddWizard {
    pub step: Step,
    pub page: Wizard,
    icon: Option<Canvas>,
    /// What cupsd's backends found, `None` while they search. [`cups::Error::Denied`] is a
    /// search without the password prompt that cupsd refused: until Search Again asks with the
    /// prompt, only the network printers show.
    local: Option<Result<Vec<Device>, cups::Error>>,
    network: Network,
    /// The rows of the found list: both of the above.
    devices: Vec<Device>,
    device: Option<usize>,
    typed: bool,
    address: String,
    /// `None` until read.
    ppds: Option<Result<Vec<Ppd>, String>>,
    asked_ppds: bool,
    search: String,
    /// The URI `search` and the driver were chosen for; another pick refills the search from
    /// that printer's maker and clears the driver.
    searched_for: Option<String>,
    shown: Vec<usize>,
    ppd: Option<String>,
    file: Option<PathBuf>,
    name: String,
    /// The URI `name` was given for; another pick replaces it with a fresh suggestion.
    named_for: Option<String>,
    location: String,
    make_default: bool,
    test_page: bool,
    pub adding: bool,
    error: String,
}

fn para(id: &'static str, y: i32, s: &str) -> Item {
    Item::label_at(id, B.x, y, B.width, s)
}

fn edit_text(form: &Form, id: &str) -> String {
    match form.item(id).map(|i| &i.control) {
        Some(Control::Edit { edit, .. }) => edit.text.clone(),
        _ => String::new(),
    }
}

fn list_selected(form: &Form, id: &str) -> Option<usize> {
    match form.item(id).map(|i| &i.control) {
        Some(Control::ListBox { list, .. }) => list.selected,
        _ => None,
    }
}

/// The URI a typed address stands for: a bare host or IP is a
/// driverless IPP printer at its usual path.
pub fn address_uri(typed: &str) -> Option<String> {
    let t = typed.trim();
    if t.is_empty() || t.contains(char::is_whitespace) {
        return None;
    }
    if t.contains("://") {
        return Some(t.to_string());
    }
    Some(format!("ipp://{t}/ipp/print"))
}

/// A queue name from what the printer calls itself, without the characters cupsd refuses.
pub fn suggest_name(from: &str) -> String {
    let words: Vec<String> = from
        .split(|c: char| !(c.is_alphanumeric() || c == '-' || c == '.'))
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect();
    let name = words.join("_");
    let name: String = name.chars().take(60).collect();
    if name.is_empty() {
        "Printer".into()
    } else {
        name
    }
}

fn free_name(base: &str, s: &Snapshot) -> String {
    let taken = |n: &str| s.printers.iter().any(|p| p.name.eq_ignore_ascii_case(n));
    if !taken(base) {
        return base.into();
    }
    (2..)
        .map(|i| format!("{base}_{i}"))
        .find(|n| !taken(n))
        .unwrap_or_else(|| base.into())
}

impl AddWizard {
    pub fn new(s: &Snapshot, icon: Option<Canvas>) -> AddWizard {
        let mut w = AddWizard {
            step: Step::Choose,
            icon,
            page: Wizard::new(
                Style::Exterior {
                    title: String::new(),
                },
                vec![],
                true,
                false,
            ),
            local: None,
            network: Network::Browsing,
            devices: Vec::new(),
            device: None,
            typed: false,
            address: String::new(),
            ppds: None,
            asked_ppds: false,
            search: String::new(),
            searched_for: None,
            shown: Vec::new(),
            ppd: None,
            file: None,
            name: String::new(),
            named_for: None,
            location: String::new(),
            make_default: s.printers.is_empty(),
            test_page: false,
            adding: false,
            error: String::new(),
        };
        w.go(Step::Choose, s);
        w
    }

    fn uri(&self) -> Option<String> {
        if self.typed {
            return address_uri(&self.address);
        }
        Some(self.devices.get(self.device?)?.uri.clone())
    }

    fn chosen_device(&self) -> Option<&Device> {
        if self.typed {
            return None;
        }
        self.devices.get(self.device?)
    }

    fn needs_driver(&self) -> bool {
        !self.uri().is_some_and(|u| cups::driverless_uri(&u))
    }

    fn driver(&self) -> Driver {
        if !self.needs_driver() {
            return Driver::Everywhere;
        }
        match (&self.file, &self.ppd) {
            (Some(f), _) => Driver::File(f.clone()),
            (None, Some(p)) => Driver::Ppd(p.clone()),
            (None, None) => Driver::Everywhere,
        }
    }

    fn interior(
        &self,
        title: &str,
        subtitle: &str,
        items: Vec<Item>,
        first: bool,
        last: bool,
    ) -> Wizard {
        Wizard::new(
            Style::Interior {
                title: title.into(),
                subtitle: subtitle.into(),
                icon: self.icon.clone(),
            },
            items,
            first,
            last,
        )
    }

    fn go(&mut self, step: Step, s: &Snapshot) {
        self.step = step;
        self.page = match step {
            Step::Choose => self.choose_page(),
            Step::Driver => self.driver_page(),
            Step::Name => self.name_page(s),
        };
        self.sync_next();
    }

    fn status(&self) -> String {
        let (n, network_only) = (self.devices.len(), self.local_pending());
        let searching = match &self.local {
            None => true,
            Some(Err(e)) if !network_only => return e.text(),
            Some(_) => self.network == Network::Browsing,
        };
        let found = match n {
            _ if searching => return "Searching for printers...".into(),
            0 => "No printers were found".to_string(),
            1 => "1 printer was found".into(),
            n => format!("{n} printers were found"),
        };
        if network_only {
            format!("{found} on the network.")
        } else {
            format!("{found}.")
        }
    }

    /// cupsd's search has not run with the password prompt yet.
    fn local_pending(&self) -> bool {
        matches!(self.local, Some(Err(cups::Error::Denied)))
    }

    fn choose_page(&self) -> Wizard {
        let labels: Vec<String> = self.devices.iter().map(Device::label).collect();
        let status = self.status();
        let list_h = 104;
        let y_status = B.y + 20 + list_h + 6;
        let y_typed = y_status + BUTTON_H + 8;
        let items = vec![
            Item::radio(FOUND, B.x, B.y, "A printer &found by this computer:", "how")
                .checked(!self.typed),
            Item::new(
                DEVICES,
                Rect::new(B.x + 18, B.y + 20, B.width - 18, list_h),
                Control::ListBox {
                    list: ListBox::new(labels, if self.typed { None } else { self.device }),
                    icons: Vec::new(),
                },
            ),
            Item::label_at(STATUS, B.x + 18, y_status + 4, B.width - 18 - 100, &status),
            Item::button_at(
                AGAIN,
                B.right() - 96,
                y_status,
                96,
                "&Search Again",
                self.local.is_some(),
            ),
            Item::radio(
                TYPED,
                B.x,
                y_typed,
                "The printer is not listed. Its &address:",
                "how",
            )
            .checked(self.typed),
            Item::edit(
                ADDRESS,
                Rect::new(B.x + 18, y_typed + 18, B.width - 18, 21),
                &self.address,
                true,
            ),
            Item::label_at(
                "hint",
                B.x + 18,
                y_typed + 44,
                B.width - 18,
                "A host name, an IP address or a URI (socket://host:9100)",
            )
            .enabled(false),
        ];
        let mut w = self.interior(
            "Choose a printer",
            "Pick a printer Reprise found, or type its address.",
            items,
            true,
            false,
        );
        if self.typed {
            w.form.focus_id(ADDRESS);
        } else {
            w.form.focus_id(DEVICES);
        }
        w
    }

    fn filter(&mut self) {
        let words: Vec<String> = self
            .search
            .split_whitespace()
            .map(str::to_lowercase)
            .collect();
        self.shown = match &self.ppds {
            Some(Ok(p)) => p
                .iter()
                .enumerate()
                .filter(|(_, d)| {
                    let mm = d.make_model.to_lowercase();
                    words.iter().all(|w| mm.contains(w.as_str()))
                })
                .map(|(i, _)| i)
                .take(MAX_DRIVERS)
                .collect(),
            _ => Vec::new(),
        };
        if self.driver_rows().1.is_none() {
            self.ppd = None;
        }
    }

    fn driver_rows(&self) -> (Vec<String>, Option<usize>) {
        let Some(Ok(p)) = &self.ppds else {
            return (Vec::new(), None);
        };
        let rows = self
            .shown
            .iter()
            .map(|&i| p[i].make_model.clone())
            .collect();
        let at = self
            .ppd
            .as_ref()
            .and_then(|n| self.shown.iter().position(|&i| p[i].name == *n));
        (rows, at)
    }

    fn driver_status(&self) -> String {
        if let Some(f) = &self.file {
            return format!("Driver file: {}", f.display());
        }
        match &self.ppds {
            None => "Reading the list of drivers...".into(),
            Some(Err(why)) => why.clone(),
            Some(Ok(_)) if self.shown.is_empty() => "No driver matches the search.".into(),
            Some(Ok(_)) => String::new(),
        }
    }

    fn driver_page(&mut self) -> Wizard {
        let uri = self.uri();
        if self.searched_for != uri {
            self.search = self.chosen_device().map(Device::maker).unwrap_or_default();
            self.searched_for = uri;
            self.ppd = None;
            self.file = None;
        }
        self.filter();
        let (rows, at) = self.driver_rows();
        let file = self.driver_status();
        let items = vec![
            Item::label_at("search_label", B.x, B.y + 4, 50, "Sea&rch:"),
            Item::edit(
                SEARCH,
                Rect::new(B.x + 52, B.y, B.width - 52, 21),
                &self.search,
                true,
            ),
            Item::new(
                DRIVERS,
                Rect::new(B.x, B.y + 28, B.width, 150),
                Control::ListBox {
                    list: ListBox::new(rows, at),
                    icons: Vec::new(),
                },
            ),
            Item::label_at(STATUS, B.x, B.y + 188, B.width - 106, &file),
            Item::button_at(DISK, B.right() - 96, B.y + 184, 96, "&Have Disk...", true),
        ];
        let mut w = self.interior(
            "Install printer software",
            "Pick the printer's maker and model, or a driver file.",
            items,
            false,
            false,
        );
        w.form.focus_id(SEARCH);
        w
    }

    fn name_page(&mut self, s: &Snapshot) -> Wizard {
        let uri = self.uri();
        if self.name.trim().is_empty() || self.named_for != uri {
            let from = match self.chosen_device() {
                Some(d) if !d.info.is_empty() => d.info.clone(),
                Some(d) => d.make_model.clone(),
                None => self
                    .address
                    .split("://")
                    .last()
                    .unwrap_or_default()
                    .split(['/', ':'])
                    .next()
                    .unwrap_or_default()
                    .to_string(),
            };
            self.name = free_name(&suggest_name(&from), s);
            self.named_for = uri;
        }
        let items = vec![
            Item::label_at("name_label", B.x, B.y + 4, 90, "Printer &name:"),
            Item::edit(
                NAME,
                Rect::new(B.x + 92, B.y, B.width - 92, 21),
                &self.name,
                true,
            ),
            Item::label_at("location_label", B.x, B.y + 34, 90, "&Location:"),
            Item::edit(
                LOCATION,
                Rect::new(B.x + 92, B.y + 30, B.width - 92, 21),
                &self.location,
                true,
            ),
            para("hint", B.y + 58, "Spaces in the name become underscores.").enabled(false),
            Item::check(DEFAULT, B.x, B.y + 90, "Set as the &default printer")
                .checked(self.make_default),
            Item::check(TEST, B.x, B.y + 114, "Print a &test page").checked(self.test_page),
            Item::label_at(ERROR, B.x, B.y + 146, B.width, &self.error),
        ];
        let mut w = self.interior(
            "Name your printer",
            "Programs show this name when you print.",
            items,
            false,
            true,
        );
        w.form.focus_id(NAME);
        w
    }

    fn can_next(&self) -> bool {
        match self.step {
            Step::Choose => self.uri().is_some(),
            Step::Driver => self.file.is_some() || self.ppd.is_some(),
            Step::Name => !self.name.trim().is_empty() && !self.adding,
        }
    }

    fn sync_next(&mut self) {
        let ok = self.can_next();
        self.page.set_next_enabled(ok);
        self.page
            .set_back_enabled(self.step != Step::Choose && !self.adding);
        self.page.set_cancel_enabled(!self.adding);
    }

    /// What cupsd's backends found. A dismissed password prompt leaves the network list, or
    /// with no avahi-daemon an empty one: asking again would bring the prompt straight back.
    pub fn devices_in(&mut self, r: cups::Result<Vec<Device>>) -> Outcome {
        self.local = Some(match r {
            Err(cups::Error::Dismissed) if self.network != Network::Off => Err(cups::Error::Denied),
            r => r,
        });
        self.found_changed()
    }

    /// avahi-daemon's network printers, `None` when it is not there.
    pub fn network_in(&mut self, found: Option<Vec<Device>>) -> Outcome {
        self.network = found.map_or(Network::Off, Network::Found);
        self.found_changed()
    }

    /// Rebuild the found list, keeping the pick. With no avahi-daemon an account cupsd refused
    /// gets the search with the password prompt right away.
    fn found_changed(&mut self) -> Outcome {
        let network = match &self.network {
            Network::Found(d) => d.as_slice(),
            _ => &[],
        };
        let found = match &self.local {
            Some(Ok(d)) => cups::merge(d, network),
            _ => cups::merge(&[], network),
        };
        let had = self.device.and_then(|i| self.devices.get(i));
        self.device = match had {
            Some(d) => found
                .iter()
                .position(|x| x.uri == d.uri)
                .or_else(|| found.iter().position(|x| cups::same(x, d))),
            None if !found.is_empty() && !self.typed => Some(0),
            None => None,
        };
        self.devices = found;
        let fallback = self.local_pending() && self.network == Network::Off;
        if fallback {
            self.local = None;
        }
        self.show_found();
        if fallback {
            Outcome::Search { prompt: true }
        } else {
            Outcome::Redraw
        }
    }

    /// Update the found list, its status and Search Again in place, so a printer coming or
    /// going leaves the focus and a typed address alone.
    fn show_found(&mut self) {
        if self.step != Step::Choose {
            return;
        }
        let labels: Vec<String> = self.devices.iter().map(Device::label).collect();
        let selected = if self.typed { None } else { self.device };
        if let Some(Control::ListBox { list, .. }) =
            self.page.form.item_mut(DEVICES).map(|i| &mut i.control)
        {
            list.top = list.top.min(labels.len().saturating_sub(1));
            list.items = labels;
            list.selected = selected;
        }
        let status = self.status();
        self.page.form.set_text(STATUS, &status);
        let again = self.local.is_some();
        self.page.form.set_enabled(AGAIN, again);
        self.sync_next();
    }

    pub fn ppds_in(&mut self, r: cups::Result<Vec<Ppd>>, s: &Snapshot) {
        self.ppds = Some(r.map_err(|e| e.text()));
        if self.step == Step::Driver {
            self.go(Step::Driver, s);
        }
    }

    pub fn file_in(&mut self, path: PathBuf, s: &Snapshot) {
        self.file = Some(path);
        self.ppd = None;
        self.go(Step::Driver, s);
    }

    /// cupsd refused the printer: the Name page says why, and Finish is
    /// live again.
    pub fn add_failed(&mut self, why: &str, s: &Snapshot) {
        self.adding = false;
        self.error = why.into();
        self.go(Step::Name, s);
    }

    /// The found list shows its highlight only while its radio is the active choice.
    fn set_typed(&mut self, typed: bool) {
        self.typed = typed;
        self.page.form.set_check(FOUND, !typed);
        self.page.form.set_check(TYPED, typed);
        if let Some(Control::ListBox { list, .. }) =
            self.page.form.item_mut(DEVICES).map(|i| &mut i.control)
        {
            list.selected = if typed { None } else { self.device };
        }
    }

    fn read(&mut self, id: &str) -> Outcome {
        let f = &self.page.form;
        match id {
            FOUND => {
                if self.device.is_none() && !self.devices.is_empty() {
                    self.device = Some(0);
                }
                self.set_typed(false);
            }
            TYPED => self.set_typed(true),
            DEVICES => {
                self.device = list_selected(f, DEVICES);
                self.set_typed(false);
            }
            ADDRESS => {
                self.address = edit_text(f, ADDRESS);
                let typed = !self.address.trim().is_empty() || self.typed;
                self.set_typed(typed);
            }
            SEARCH => {
                self.search = edit_text(f, SEARCH);
                self.filter();
                let (rows, at) = self.driver_rows();
                if let Some(Control::ListBox { list, .. }) =
                    self.page.form.item_mut(DRIVERS).map(|i| &mut i.control)
                {
                    *list = ListBox::new(rows, at);
                }
                let status = self.driver_status();
                self.page.form.set_text(STATUS, &status);
            }
            DRIVERS => {
                let at = list_selected(f, DRIVERS);
                if let (Some(Ok(p)), Some(i)) = (&self.ppds, at.and_then(|i| self.shown.get(i))) {
                    self.ppd = Some(p[*i].name.clone());
                    self.file = None;
                    self.page.form.set_text(STATUS, "");
                }
            }
            NAME => self.name = edit_text(f, NAME),
            LOCATION => self.location = edit_text(f, LOCATION),
            DEFAULT => self.make_default = f.checked(DEFAULT).unwrap_or(false),
            TEST => self.test_page = f.checked(TEST).unwrap_or(false),
            _ => {}
        }
        self.sync_next();
        Outcome::Redraw
    }

    fn finish(&mut self, s: &Snapshot) -> Outcome {
        let Some(uri) = self.uri() else {
            return Outcome::None;
        };
        let name = match check_name("", &self.name, s) {
            Ok(Some(n)) => n,
            Ok(None) => return Outcome::None,
            Err(why) => {
                self.error = why;
                self.page.form.set_text(ERROR, &self.error);
                return Outcome::Redraw;
            }
        };
        self.adding = true;
        self.error = "Adding the printer...".into();
        self.page.form.set_text(ERROR, &self.error);
        self.sync_next();
        let info = match self.chosen_device() {
            Some(d) if !d.info.is_empty() => d.info.clone(),
            Some(d) => d.make_model.clone(),
            None => String::new(),
        };
        Outcome::Add {
            printer: NewPrinter {
                name,
                device_uri: uri,
                driver: self.driver(),
                info,
                location: self.location.trim().to_string(),
            },
            default: self.make_default,
            test_page: self.test_page,
        }
    }

    pub fn event(&mut self, ev: form::Event, s: &Snapshot) -> Outcome {
        if self.adding && !matches!(ev, form::Event::Redraw) {
            return Outcome::None;
        }
        match self.page.event(ev) {
            wizard::Event::None => Outcome::None,
            wizard::Event::Redraw => Outcome::Redraw,
            wizard::Event::Cancel => Outcome::Close,
            wizard::Event::Back => {
                let to = match self.step {
                    Step::Name if self.needs_driver() => Step::Driver,
                    _ => Step::Choose,
                };
                self.error.clear();
                self.go(to, s);
                Outcome::Redraw
            }
            wizard::Event::Next | wizard::Event::Finish if !self.can_next() => Outcome::None,
            wizard::Event::Next => match self.step {
                Step::Choose if self.needs_driver() => {
                    self.go(Step::Driver, s);
                    if self.asked_ppds {
                        Outcome::Redraw
                    } else {
                        self.asked_ppds = true;
                        Outcome::Drivers
                    }
                }
                Step::Choose | Step::Driver => {
                    self.go(Step::Name, s);
                    Outcome::Redraw
                }
                Step::Name => Outcome::None,
            },
            wizard::Event::Finish => self.finish(s),
            wizard::Event::Page(form::Event::Pressed(AGAIN)) => {
                self.local = None;
                self.show_found();
                Outcome::Search { prompt: true }
            }
            wizard::Event::Page(form::Event::Pressed(DISK)) => Outcome::HaveDisk,
            wizard::Event::Page(form::Event::Changed(id)) => self.read(id),
            wizard::Event::Page(_) => Outcome::Redraw,
        }
    }

    pub fn draw(&self, c: &mut Canvas, p: &Paint, text: &reprisekit::text::TextRenderer, px: u32) {
        wizard::draw(c, text, p, &self.page, true, px);
    }

    pub fn access(&self, tree: &mut Tree, text: &reprisekit::text::TextRenderer, px: u32) {
        if let Some(f) = self.page.access(tree, ROOT, text, px) {
            tree.set_focus(f);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reprisekit::textedit::TextEdit;

    fn type_into(w: &mut AddWizard, id: &'static str, s: &str, snap: &Snapshot) -> Outcome {
        if let Some(Control::Edit { edit, .. }) = w.page.form.item_mut(id).map(|i| &mut i.control) {
            *edit = TextEdit::new(s, false);
        }
        w.event(form::Event::Changed(id), snap)
    }
    use crate::cups::Printer;

    fn snap() -> Snapshot {
        Snapshot {
            running: true,
            printers: vec![Printer {
                name: "Office_Jet".into(),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    fn found() -> Vec<Device> {
        vec![
            Device {
                uri: "dnssd://Office%20Jet._ipp._tcp.local/?uuid=1".into(),
                info: "Office Jet".into(),
                make_model: "HP OfficeJet 5200".into(),
                class: "network".into(),
                device_id: String::new(),
            },
            Device {
                uri: "usb://Brother/HL-1110?serial=9".into(),
                info: "Brother HL-1110".into(),
                make_model: "Generic HL-1110 series".into(),
                class: "direct".into(),
                device_id: "MFG:Brother;MDL:HL-1110 series;".into(),
            },
        ]
    }

    fn ppds() -> Vec<Ppd> {
        vec![
            Ppd {
                name: "drv:///brlaser.drv/br1110.ppd".into(),
                make: "Brother".into(),
                make_model: "Brother HL-1110 series, using brlaser".into(),
            },
            Ppd {
                name: "lsb/usr/hp/hp-laserjet.ppd".into(),
                make: "HP".into(),
                make_model: "HP LaserJet".into(),
            },
        ]
    }

    fn pick_device(w: &mut AddWizard, i: usize, s: &Snapshot) {
        if let Some(Control::ListBox { list, .. }) =
            w.page.form.item_mut(DEVICES).map(|it| &mut it.control)
        {
            list.selected = Some(i);
        }
        w.event(form::Event::Changed(DEVICES), s);
    }

    #[test]
    fn a_driverless_printer_skips_the_driver_page() {
        let s = snap();
        let mut w = AddWizard::new(&s, None);
        assert_eq!(w.step, Step::Choose);
        w.devices_in(Ok(found()));
        assert_eq!(w.device, Some(0), "the first found printer is picked");
        assert_eq!(
            w.event(form::Event::Pressed(wizard::NEXT), &s),
            Outcome::Redraw
        );
        assert_eq!(w.step, Step::Name);
        assert_eq!(w.name, "Office_Jet_2");
        match w.event(form::Event::Pressed(wizard::NEXT), &s) {
            Outcome::Add {
                printer,
                default,
                test_page,
            } => {
                assert_eq!(printer.driver, Driver::Everywhere);
                assert_eq!(printer.device_uri, found()[0].uri);
                assert_eq!(printer.info, "Office Jet");
                assert!(!default, "there is a printer already");
                assert!(!test_page);
            }
            other => panic!("{other:?}"),
        }
        assert!(w.adding);
        assert_eq!(
            w.event(form::Event::Pressed(wizard::BACK), &s),
            Outcome::None
        );
        w.add_failed("Bad driver.", &s);
        assert!(!w.adding);
        assert_eq!(w.error, "Bad driver.");
    }

    #[test]
    fn a_usb_printer_picks_a_driver_from_the_search() {
        let s = snap();
        let mut w = AddWizard::new(&s, None);
        w.devices_in(Ok(found()));
        pick_device(&mut w, 1, &s);
        assert_eq!(
            w.event(form::Event::Pressed(wizard::NEXT), &s),
            Outcome::Drivers,
            "the first visit reads the drivers"
        );
        assert_eq!(w.step, Step::Driver);
        assert_eq!(w.search, "Brother", "the search starts on the make");
        w.ppds_in(Ok(ppds()), &s);
        assert_eq!(w.shown, [0]);
        assert!(
            matches!(
                w.event(form::Event::Pressed(wizard::NEXT), &s),
                Outcome::None
            ),
            "Next waits for a driver"
        );
        if let Some(Control::ListBox { list, .. }) =
            w.page.form.item_mut(DRIVERS).map(|it| &mut it.control)
        {
            list.selected = Some(0);
        }
        w.event(form::Event::Changed(DRIVERS), &s);
        assert_eq!(w.ppd.as_deref(), Some("drv:///brlaser.drv/br1110.ppd"));
        type_into(&mut w, SEARCH, "laserjet", &s);
        assert_eq!(w.shown, [1]);
        if let Some(Control::ListBox { list, .. }) =
            w.page.form.item_mut(DRIVERS).map(|it| &mut it.control)
        {
            list.selected = Some(0);
        }
        w.event(form::Event::Changed(DRIVERS), &s);
        w.event(form::Event::Pressed(wizard::NEXT), &s);
        assert_eq!(w.step, Step::Name);
        assert_eq!(w.driver(), Driver::Ppd("lsb/usr/hp/hp-laserjet.ppd".into()));
        w.event(form::Event::Pressed(wizard::BACK), &s);
        assert_eq!(w.step, Step::Driver);
        assert_eq!(w.event(form::Event::Pressed(DISK), &s), Outcome::HaveDisk);
        w.file_in("/tmp/x.ppd".into(), &s);
        assert_eq!(w.driver(), Driver::File("/tmp/x.ppd".into()));
    }

    #[test]
    fn a_typed_address_is_a_uri() {
        assert_eq!(
            address_uri("10.0.0.7").as_deref(),
            Some("ipp://10.0.0.7/ipp/print")
        );
        assert_eq!(
            address_uri(" socket://10.0.0.7:9100 ").as_deref(),
            Some("socket://10.0.0.7:9100")
        );
        assert_eq!(address_uri(""), None);
        assert_eq!(address_uri("a b"), None);
        let s = snap();
        let mut w = AddWizard::new(&s, None);
        w.devices_in(Ok(Vec::new()));
        type_into(&mut w, ADDRESS, "printer.lan", &s);
        assert!(w.typed);
        w.event(form::Event::Pressed(wizard::NEXT), &s);
        assert_eq!(w.step, Step::Name);
        assert_eq!(w.name, "printer.lan");
        type_into(&mut w, NAME, "a/b", &s);
        assert_eq!(
            w.event(form::Event::Pressed(wizard::NEXT), &s),
            Outcome::Redraw
        );
        assert!(!w.adding);
        assert!(w.error.contains("cannot contain"));
    }

    #[test]
    fn the_address_radio_clears_the_found_highlight() {
        let s = snap();
        let mut w = AddWizard::new(&s, None);
        w.devices_in(Ok(found()));
        pick_device(&mut w, 1, &s);
        type_into(&mut w, ADDRESS, "10.0.0.7", &s);
        w.event(form::Event::Changed(TYPED), &s);
        assert!(w.typed);
        assert_eq!(list_selected(&w.page.form, DEVICES), None);
        assert_eq!(w.page.form.checked(FOUND), Some(false));
        assert_eq!(w.uri().as_deref(), Some("ipp://10.0.0.7/ipp/print"));

        w.event(form::Event::Changed(FOUND), &s);
        assert!(!w.typed);
        assert_eq!(list_selected(&w.page.form, DEVICES), Some(1));
        assert_eq!(w.uri(), Some(found()[1].uri.clone()));

        w.event(form::Event::Changed(TYPED), &s);
        pick_device(&mut w, 0, &s);
        assert!(!w.typed);
        assert_eq!(w.page.form.checked(FOUND), Some(true));
        assert_eq!(w.page.form.checked(TYPED), Some(false));
        assert_eq!(list_selected(&w.page.form, DEVICES), Some(0));
        assert_eq!(w.uri(), Some(found()[0].uri.clone()));
    }

    #[test]
    fn the_suggested_name_follows_the_pick_and_a_typed_name_stays_with_it() {
        let s = snap();
        let mut w = AddWizard::new(&s, None);
        w.devices_in(Ok(found()));
        w.event(form::Event::Pressed(wizard::NEXT), &s);
        assert_eq!(w.name, "Office_Jet_2");

        w.event(form::Event::Pressed(wizard::BACK), &s);
        type_into(&mut w, ADDRESS, "printer.lan", &s);
        w.event(form::Event::Pressed(wizard::NEXT), &s);
        assert_eq!(w.step, Step::Name);
        assert_eq!(w.name, "printer.lan", "another pick gets its own name");

        type_into(&mut w, NAME, "Hall", &s);
        w.event(form::Event::Pressed(wizard::BACK), &s);
        w.event(form::Event::Pressed(wizard::NEXT), &s);
        assert_eq!(
            w.name, "Hall",
            "a typed name stays while the pick is the same"
        );
        assert_eq!(edit_text(&w.page.form, NAME), "Hall");

        w.event(form::Event::Pressed(wizard::BACK), &s);
        type_into(&mut w, ADDRESS, "attic.lan", &s);
        w.event(form::Event::Pressed(wizard::NEXT), &s);
        assert_eq!(w.name, "attic.lan", "a new address replaces the typed name");
    }

    #[test]
    fn the_driver_search_follows_the_pick_and_a_typed_search_stays_with_it() {
        let s = snap();
        let mut w = AddWizard::new(&s, None);
        let mut devices = found();
        devices.push(Device {
            uri: "usb://Epson/L3150?serial=4".into(),
            info: "Epson L3150".into(),
            make_model: "Epson L3150 Series".into(),
            class: "direct".into(),
            device_id: "MFG:EPSON;MDL:L3150 Series;".into(),
        });
        w.devices_in(Ok(devices));
        pick_device(&mut w, 1, &s);
        w.event(form::Event::Pressed(wizard::NEXT), &s);
        w.ppds_in(Ok(ppds()), &s);
        assert_eq!(w.search, "Brother");

        w.event(form::Event::Pressed(wizard::BACK), &s);
        pick_device(&mut w, 2, &s);
        w.event(form::Event::Pressed(wizard::NEXT), &s);
        assert_eq!(w.search, "EPSON", "another pick searches its own maker");

        type_into(&mut w, SEARCH, "laserjet", &s);
        w.event(form::Event::Pressed(wizard::BACK), &s);
        w.event(form::Event::Pressed(wizard::NEXT), &s);
        assert_eq!(
            w.search, "laserjet",
            "a typed search stays while the pick is the same"
        );
        assert_eq!(edit_text(&w.page.form, SEARCH), "laserjet");

        w.event(form::Event::Pressed(wizard::BACK), &s);
        pick_device(&mut w, 1, &s);
        w.event(form::Event::Pressed(wizard::NEXT), &s);
        assert_eq!(w.search, "Brother", "a new pick replaces the typed search");
    }

    #[test]
    fn a_driver_is_dropped_when_the_pick_changes_or_the_search_hides_it() {
        let s = snap();
        let mut w = AddWizard::new(&s, None);
        w.devices_in(Ok(found()));
        pick_device(&mut w, 1, &s);
        w.event(form::Event::Pressed(wizard::NEXT), &s);
        w.ppds_in(Ok(ppds()), &s);
        if let Some(Control::ListBox { list, .. }) =
            w.page.form.item_mut(DRIVERS).map(|it| &mut it.control)
        {
            list.selected = Some(0);
        }
        w.event(form::Event::Changed(DRIVERS), &s);
        assert_eq!(w.ppd.as_deref(), Some("drv:///brlaser.drv/br1110.ppd"));

        type_into(&mut w, SEARCH, "laserjet", &s);
        assert_eq!(w.ppd, None, "a driver the search hides is not picked");
        assert_eq!(
            w.event(form::Event::Pressed(wizard::NEXT), &s),
            Outcome::None
        );

        type_into(&mut w, SEARCH, "brother", &s);
        w.event(form::Event::Pressed(wizard::BACK), &s);
        w.event(form::Event::Pressed(wizard::NEXT), &s);
        assert_eq!(w.ppd, None);
        w.file_in("/tmp/brother.ppd".into(), &s);
        w.event(form::Event::Pressed(wizard::BACK), &s);
        w.event(form::Event::Pressed(wizard::NEXT), &s);
        assert_eq!(
            w.driver(),
            Driver::File("/tmp/brother.ppd".into()),
            "the driver stays while the pick is the same"
        );

        w.event(form::Event::Pressed(wizard::BACK), &s);
        type_into(&mut w, ADDRESS, "socket://10.0.0.7:9100", &s);
        w.event(form::Event::Pressed(wizard::NEXT), &s);
        assert_eq!(w.step, Step::Driver);
        assert_eq!(
            (w.ppd.as_ref(), w.file.as_ref()),
            (None, None),
            "another pick starts with no driver"
        );
        assert_eq!(
            w.event(form::Event::Pressed(wizard::NEXT), &s),
            Outcome::None
        );
    }

    fn network(name: &str) -> Device {
        crate::avahi::device(name, "_ipp._tcp", "local", &[], 0).expect("a network printer")
    }

    fn status(w: &AddWizard) -> String {
        match w.page.form.item(STATUS).map(|i| &i.control) {
            Some(Control::Label { text, .. }) => text.clone(),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn outside_the_admin_group_a_network_printer_is_added_with_no_prompt_before_finish() {
        let s = snap();
        let mut w = AddWizard::new(&s, None);
        assert_eq!(status(&w), "Searching for printers...");
        assert_eq!(w.devices_in(Err(cups::Error::Denied)), Outcome::Redraw);
        assert_eq!(status(&w), "Searching for printers...");
        assert_eq!(w.network_in(Some(vec![network("Lab")])), Outcome::Redraw);
        assert_eq!(status(&w), "1 printer was found on the network.");
        assert_eq!(w.device, Some(0));
        assert_eq!(
            w.event(form::Event::Pressed(wizard::NEXT), &s),
            Outcome::Redraw
        );
        assert_eq!(w.step, Step::Name);
        match w.event(form::Event::Pressed(wizard::NEXT), &s) {
            Outcome::Add { printer, .. } => {
                assert_eq!(printer.device_uri, "dnssd://Lab._ipp._tcp.local/");
                assert_eq!(printer.driver, Driver::Everywhere);
                assert_eq!(printer.info, "Lab");
            }
            other => panic!("{other:?}"),
        }

        let mut w = AddWizard::new(&s, None);
        assert_eq!(w.network_in(Some(Vec::new())), Outcome::Redraw);
        assert_eq!(w.devices_in(Err(cups::Error::Denied)), Outcome::Redraw);
        assert_eq!(status(&w), "No printers were found on the network.");
    }

    #[test]
    fn without_avahi_a_refused_search_asks_with_the_prompt_at_once() {
        let s = snap();
        let mut w = AddWizard::new(&s, None);
        assert_eq!(w.devices_in(Err(cups::Error::Denied)), Outcome::Redraw);
        assert_eq!(w.network_in(None), Outcome::Search { prompt: true });
        assert_eq!(status(&w), "Searching for printers...");
        assert_eq!(w.devices_in(Ok(found())), Outcome::Redraw);
        assert_eq!(status(&w), "2 printers were found.");

        let mut w = AddWizard::new(&s, None);
        assert_eq!(w.network_in(None), Outcome::Redraw);
        assert_eq!(
            w.devices_in(Err(cups::Error::Denied)),
            Outcome::Search { prompt: true }
        );

        let mut w = AddWizard::new(&s, None);
        assert_eq!(w.network_in(None), Outcome::Redraw);
        assert_eq!(
            w.devices_in(Ok(found())),
            Outcome::Redraw,
            "an admin needs no prompt"
        );

        let mut w = AddWizard::new(&s, None);
        w.network_in(None);
        w.devices_in(Err(cups::Error::Denied));
        assert_eq!(
            w.devices_in(Err(cups::Error::Dismissed)),
            Outcome::Redraw,
            "a dismissed prompt is not asked again"
        );
        assert_eq!(
            w.event(form::Event::Pressed(AGAIN), &s),
            Outcome::Search { prompt: true }
        );
    }

    #[test]
    fn search_again_asks_with_the_prompt_and_a_dismissed_prompt_keeps_the_network_list() {
        let s = snap();
        let mut w = AddWizard::new(&s, None);
        w.network_in(Some(vec![network("Lab")]));
        w.devices_in(Err(cups::Error::Denied));
        assert_eq!(
            w.event(form::Event::Pressed(AGAIN), &s),
            Outcome::Search { prompt: true }
        );
        assert_eq!(status(&w), "Searching for printers...");
        assert_eq!(
            w.devices.len(),
            1,
            "the network list stays while cupsd searches"
        );
        assert_eq!(w.devices_in(Err(cups::Error::Dismissed)), Outcome::Redraw);
        assert_eq!(status(&w), "1 printer was found on the network.");

        w.event(form::Event::Pressed(AGAIN), &s);
        w.devices_in(Ok(found()));
        let uris: Vec<&str> = w.devices.iter().map(|d| d.uri.as_str()).collect();
        assert_eq!(
            uris,
            [
                "dnssd://Lab._ipp._tcp.local/",
                found()[0].uri.as_str(),
                found()[1].uri.as_str()
            ]
        );
        assert_eq!(status(&w), "3 printers were found.");
    }

    #[test]
    fn the_pick_follows_its_printer_as_the_network_list_changes() {
        let s = snap();
        let mut w = AddWizard::new(&s, None);
        w.devices_in(Ok(found()));
        w.network_in(Some(vec![network("Office Jet")]));
        assert_eq!(
            w.devices.len(),
            2,
            "cupsd's own entry for a network printer stays"
        );
        pick_device(&mut w, 1, &s);
        w.network_in(Some(vec![network("Attic"), network("Office Jet")]));
        assert_eq!(w.devices[0].info, "Attic");
        assert_eq!(w.device, Some(2));
        assert_eq!(list_selected(&w.page.form, DEVICES), Some(2));
        assert_eq!(w.uri(), Some(found()[1].uri.clone()));

        pick_device(&mut w, 0, &s);
        w.network_in(Some(Vec::new()));
        assert_eq!(w.device, None, "a printer that went is not picked");
        assert!(!w.can_next());

        let mut w = AddWizard::new(&s, None);
        w.network_in(Some(vec![network("Attic"), network("Lab")]));
        pick_device(&mut w, 1, &s);
        let lab = Device {
            uri: "ipps://Lab._ipps._tcp.local/".into(),
            info: "Example Laser (driverless)".into(),
            class: "network".into(),
            ..Default::default()
        };
        w.devices_in(Ok(vec![lab.clone()]));
        assert_eq!(w.devices.len(), 2);
        assert_eq!(
            w.uri(),
            Some(lab.uri),
            "the pick moves to cupsd's entry for the same printer"
        );
    }

    #[test]
    fn names_come_from_what_the_printer_calls_itself() {
        assert_eq!(
            suggest_name("HP LaserJet 400 (M401)"),
            "HP_LaserJet_400_M401"
        );
        assert_eq!(suggest_name("  "), "Printer");
        let s = snap();
        assert_eq!(free_name("office_jet", &s), "office_jet_2");
        assert_eq!(free_name("Laser", &s), "Laser");
    }

    #[test]
    #[ignore]
    fn render_wizard() {
        use reprisekit::text::TextRenderer;
        use reprisekit::theme::{FontConfig, Theme};
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let paint = Paint {
            s: 1,
            ..theme.paint()
        };
        let s = snap();
        let mut w = AddWizard::new(&s, None);
        w.devices_in(Ok(found()));
        w.network_in(Some(Vec::new()));
        pick_device(&mut w, 1, &s);
        let mut shots = Vec::new();
        let mut shot = |w: &AddWizard| {
            let (cw, ch) = wizard::CLIENT;
            let mut c = Canvas::new(cw as u32, ch as u32);
            w.draw(&mut c, &paint, &text, theme.fonts.size_px);
            shots.push(c);
        };
        shot(&w);
        w.event(form::Event::Pressed(wizard::NEXT), &s);
        w.ppds_in(Ok(ppds()), &s);
        shot(&w);
        w.file_in("/tmp/brother.ppd".into(), &s);
        w.event(form::Event::Pressed(wizard::NEXT), &s);
        shot(&w);
        for (i, c) in shots.iter().enumerate() {
            reprisekit::raster::write_preview(c, &format!("printers-wizard-{i}.png"));
        }
    }
}
