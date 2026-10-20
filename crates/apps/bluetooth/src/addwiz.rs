//! The Add Bluetooth Device Wizard. Nothing here reads the bus: the host feeds it snapshots,
//! the agent's requests and the pairing's result, and turns the [`Outcome`]s into commands.

use repriseapp::Ctx;
use reprisekit::access::{ROOT, Tree};
use reprisekit::art::bticon;
use reprisekit::classic::widgets::{BUTTON_H, LINE_H};
use reprisekit::controls::form::{self, Item};
use reprisekit::controls::metrics::ControlMetrics;
use reprisekit::controls::tilelist::{self, Entry, Press, TILE_H, Tile, TileList};
use reprisekit::controls::wizard::{self, EXTERIOR_BODY, INTERIOR_BODY, Style, Wizard};
use reprisekit::look::Paint;
use reprisekit::pointer::Click;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::{TextRenderer, wrap};
use reprisekit::theme::Theme;

use crate::bz::{AgentRequest, Ask, Device, Snapshot};
use crate::icons::Icons;
use crate::passkey;

const READY: &str = "ready";
const STATUS: &str = "status";
const AGAIN: &str = "again";
const AGAIN_W: i32 = 100;

pub const LIST: Rect = Rect::new(
    INTERIOR_BODY.x,
    INTERIOR_BODY.y,
    INTERIOR_BODY.width,
    INTERIOR_BODY.height - BUTTON_H - 8,
);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Welcome,
    Search,
    Pairing,
    Finish,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    None,
    Redraw,
    Close,
    StartDiscovery,
    StopDiscovery,
    SearchAgain,
    Pair(String),
    Cancel {
        discovery: bool,
        pairing: Option<String>,
    },
}

pub struct AddWizard {
    pub step: Step,
    pub page: Wizard,
    pub found: Vec<Device>,
    pub list: TileList,
    pub target: Option<Device>,
    pub ask: Option<AgentRequest>,
    discovering: bool,
    /// Finish: Ok(why the connect failed, if it did) when the device was
    /// added, Err(reason) when it was not.
    result: Option<Result<Option<String>, String>>,
    ready: bool,
}

fn para(
    id: &'static str,
    s: &str,
    body: Rect,
    y: i32,
    text: &TextRenderer,
    forms: &Theme,
) -> (Item, i32) {
    let lines = wrap(text, s, forms.text.len(body.width), forms.fonts.size_px);
    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    let item = Item::lines_at(id, body.x, y, body.width, &refs);
    (item, y + LINE_H * lines.len() as i32 + 12)
}

fn header_icon() -> Option<Canvas> {
    Some(bticon::draw32(true))
}

impl AddWizard {
    pub fn new(text: &TextRenderer, forms: &Theme) -> AddWizard {
        let mut w = AddWizard {
            step: Step::Welcome,
            page: Wizard::new(
                Style::Exterior {
                    title: String::new(),
                },
                vec![],
                true,
                false,
            ),
            found: Vec::new(),
            list: TileList::new(TILE_H),
            target: None,
            ask: None,
            discovering: false,
            result: None,
            ready: false,
        };
        w.go(Step::Welcome, text, forms);
        w
    }

    /// The found-devices list on the page as last drawn: it grows with the page.
    pub fn list_rect(&self) -> Rect {
        self.page.form.units().rect(LIST)
    }

    pub fn target_path(&self) -> Option<&str> {
        self.target.as_ref().map(|d| d.path.as_str())
    }

    fn target_name(&self) -> String {
        self.target
            .as_ref()
            .map_or_else(|| "The device".to_string(), |d| d.alias.clone())
    }

    pub fn reason(&self) -> Option<&str> {
        match &self.result {
            Some(Err(r)) => Some(r),
            _ => None,
        }
    }

    pub fn status(&self) -> &'static str {
        if self.discovering {
            "Searching for Bluetooth devices..."
        } else if self.found.is_empty() {
            "No devices were found."
        } else {
            "Search finished."
        }
    }

    fn go(&mut self, step: Step, text: &TextRenderer, forms: &Theme) {
        self.step = step;
        self.page = match step {
            Step::Welcome => {
                let b = EXTERIOR_BODY;
                let (intro, y) = para(
                    "intro",
                    "This wizard adds a Bluetooth device to this computer: headphones, a speaker, a keyboard, a mouse, a game controller or a phone.",
                    b,
                    b.y,
                    text,
                    forms,
                );
                let (how, y) = para(
                    "how",
                    "Turn the device on and make it discoverable. Its manual says how; many devices have a button to hold down until a light flashes.",
                    b,
                    y,
                    text,
                    forms,
                );
                let check =
                    Item::check(READY, b.x, y, "&My device is set up and ready to be found.")
                        .checked(self.ready);
                let mut w = Wizard::new(
                    Style::Exterior {
                        title: "Welcome to the Add Bluetooth Device Wizard".into(),
                    },
                    vec![intro, how, check],
                    true,
                    false,
                );
                w.set_next_enabled(self.ready);
                w
            }
            Step::Search => {
                let y = LIST.bottom() + 8;
                let items = vec![
                    Item::label_at(STATUS, LIST.x, y + 5, LIST.width - AGAIN_W - 8, ""),
                    Item::button_at(
                        AGAIN,
                        LIST.right() - AGAIN_W,
                        y,
                        AGAIN_W,
                        "&Search Again",
                        true,
                    ),
                ];
                Wizard::new(
                    Style::Interior {
                        title: "Select the Bluetooth device that you want to add.".into(),
                        subtitle: "If it is not listed, make sure it is on and discoverable, then click Search Again.".into(),
                        icon: header_icon(),
                    },
                    items,
                    false,
                    false,
                )
            }
            Step::Pairing => {
                let name = self.target_name();
                let items = match &self.ask {
                    Some(req) => passkey::items(&req.ask, &name, INTERIOR_BODY, text, forms),
                    None => {
                        let b = INTERIOR_BODY;
                        vec![
                            para(
                                "connecting",
                                &format!("Connecting to {name}..."),
                                b,
                                b.y,
                                text,
                                forms,
                            )
                            .0,
                        ]
                    }
                };
                let mut w = Wizard::new(
                    Style::Interior {
                        title: format!("Adding {name}"),
                        subtitle: "Follow any steps below to finish adding the device.".into(),
                        icon: header_icon(),
                    },
                    items,
                    false,
                    false,
                );
                w.set_back_enabled(false);
                // A typed code goes straight into the field.
                if self
                    .ask
                    .as_ref()
                    .is_some_and(|r| matches!(r.ask, Ask::PinCode | Ask::Passkey))
                {
                    w.form.focus_id(passkey::CODE);
                }
                let ready = self
                    .ask
                    .as_ref()
                    .is_some_and(|r| passkey::ready(&r.ask, &w.form));
                w.set_next_enabled(ready);
                w
            }
            Step::Finish => {
                let name = self.target_name();
                let lines: Vec<String> = match &self.result {
                    Some(Ok(None)) => vec![
                        format!("{name} was added and is connected."),
                        "It is ready to use.".into(),
                    ],
                    Some(Ok(Some(e))) => vec![
                        format!("{name} was added, but it could not connect: {e}"),
                        "Turn it on and connect it from the Bluetooth Devices window.".into(),
                    ],
                    Some(Err(reason)) => vec![
                        format!("{name} could not be added."),
                        reason.clone(),
                        "Click Back to search again, or Finish to close this wizard.".into(),
                    ],
                    None => vec![],
                };
                let b = EXTERIOR_BODY;
                let mut items = Vec::new();
                let mut y = b.y;
                for (i, l) in lines.iter().enumerate() {
                    const IDS: [&str; 3] = ["finish0", "finish1", "finish2"];
                    let (item, next) = para(IDS[i.min(2)], l, b, y, text, forms);
                    items.push(item);
                    y = next;
                }
                let failed = matches!(self.result, Some(Err(_)));
                let mut w = Wizard::new(
                    Style::Exterior {
                        title: "Completing the Add Bluetooth Device Wizard".into(),
                    },
                    items,
                    !failed,
                    true,
                );
                w.set_cancel_enabled(false);
                w
            }
        };
        if step == Step::Search {
            self.sync_search();
        }
    }

    fn sync_search(&mut self) {
        let status = self.status();
        self.page.form.set_text(STATUS, status);
        let picked = self.list.selected_row().is_some();
        self.page.set_next_enabled(picked);
    }

    fn fail(&mut self, reason: &str, text: &TextRenderer, forms: &Theme) {
        self.ask = None;
        self.result = Some(Err(reason.to_string()));
        self.go(Step::Finish, text, forms);
    }

    pub fn refresh(&mut self, s: &Snapshot, text: &TextRenderer, forms: &Theme) {
        if matches!(self.step, Step::Search | Step::Pairing) {
            match &s.adapter {
                None => return self.fail("The Bluetooth adapter was removed.", text, forms),
                Some(a) if !a.powered => {
                    return self.fail("Bluetooth was turned off.", text, forms);
                }
                _ => {}
            }
        }
        self.discovering = s.adapter.as_ref().is_some_and(|a| a.discovering);
        match self.step {
            Step::Search => {
                let keep = self
                    .list
                    .selected_row()
                    .and_then(|i| self.found.get(i))
                    .map(|d| d.path.clone());
                self.found = s
                    .devices
                    .iter()
                    .filter(|d| !d.paired && d.rssi.is_some() && d.name.is_some())
                    .cloned()
                    .collect();
                self.list.entries = (0..self.found.len()).map(Entry::Row).collect();
                self.list
                    .select_row(keep.and_then(|p| self.found.iter().position(|d| d.path == p)));
                self.list.clamp(self.list_rect(), self.page.form.units());
                self.sync_search();
            }
            Step::Pairing => {
                if let Some(d) = self
                    .target_path()
                    .and_then(|p| s.devices.iter().find(|d| d.path == p))
                {
                    self.target = Some(d.clone());
                }
            }
            Step::Welcome | Step::Finish => {}
        }
    }

    pub fn agent(
        &mut self,
        req: AgentRequest,
        text: &TextRenderer,
        forms: &Theme,
    ) -> Result<(), AgentRequest> {
        if self.step != Step::Pairing || self.target_path() != Some(req.device.as_str()) {
            return Err(req);
        }
        self.ask = Some(req);
        self.go(Step::Pairing, text, forms);
        Ok(())
    }

    /// BlueZ called off the request it is waiting on. True when the
    /// wizard held it; a shown code, which BlueZ does not wait on, is
    /// not what the call is about and stays up.
    pub fn agent_cancel(&mut self, text: &TextRenderer, forms: &Theme) -> bool {
        if !self.ask.as_ref().is_some_and(|r| r.done.is_some()) {
            return false;
        }
        self.ask = None;
        if self.step == Step::Pairing {
            self.go(Step::Pairing, text, forms);
        }
        true
    }

    pub fn paired(
        &mut self,
        device: &str,
        connect_error: Option<String>,
        text: &TextRenderer,
        forms: &Theme,
    ) {
        if self.step == Step::Pairing && self.target_path() == Some(device) {
            self.ask = None;
            self.result = Some(Ok(connect_error));
            self.go(Step::Finish, text, forms);
        }
    }

    pub fn pair_failed(
        &mut self,
        device: &str,
        reason: String,
        text: &TextRenderer,
        forms: &Theme,
    ) {
        if self.step == Step::Pairing && self.target_path() == Some(device) {
            self.fail(&reason, text, forms);
        }
    }

    fn next(&mut self, text: &TextRenderer, forms: &Theme) -> Outcome {
        match self.step {
            Step::Welcome => {
                self.go(Step::Search, text, forms);
                Outcome::StartDiscovery
            }
            Step::Search => {
                let Some(d) = self
                    .list
                    .selected_row()
                    .and_then(|i| self.found.get(i))
                    .cloned()
                else {
                    return Outcome::None;
                };
                let path = d.path.clone();
                self.target = Some(d);
                self.result = None;
                self.go(Step::Pairing, text, forms);
                Outcome::Pair(path)
            }
            Step::Pairing => {
                let Some(req) = self.ask.as_mut() else {
                    return Outcome::None;
                };
                let Some(a) = passkey::answer(&req.ask, &self.page.form) else {
                    return Outcome::None;
                };
                req.answer(a);
                self.ask = None;
                self.go(Step::Pairing, text, forms);
                Outcome::Redraw
            }
            Step::Finish => Outcome::Close,
        }
    }

    pub fn event(&mut self, ev: form::Event, text: &TextRenderer, forms: &Theme) -> Outcome {
        match self.page.event(ev) {
            wizard::Event::None => Outcome::None,
            wizard::Event::Redraw => Outcome::Redraw,
            wizard::Event::Page(form::Event::Changed(READY)) => {
                self.ready = self.page.form.checked(READY).unwrap_or(false);
                self.page.set_next_enabled(self.ready);
                Outcome::Redraw
            }
            wizard::Event::Page(form::Event::Pressed(AGAIN)) => {
                self.found.clear();
                self.list = TileList::new(TILE_H);
                self.discovering = true;
                self.sync_search();
                Outcome::SearchAgain
            }
            wizard::Event::Page(form::Event::Changed(_)) => {
                if let Some(req) = &self.ask {
                    let ready = passkey::ready(&req.ask, &self.page.form);
                    self.page.set_next_enabled(ready);
                }
                Outcome::Redraw
            }
            wizard::Event::Page(_) => Outcome::Redraw,
            wizard::Event::Back => match self.step {
                Step::Search => {
                    self.go(Step::Welcome, text, forms);
                    Outcome::StopDiscovery
                }
                Step::Finish if self.reason().is_some() => {
                    self.result = None;
                    self.target = None;
                    self.found.clear();
                    self.list = TileList::new(TILE_H);
                    self.go(Step::Search, text, forms);
                    Outcome::StartDiscovery
                }
                _ => Outcome::None,
            },
            wizard::Event::Next => self.next(text, forms),
            wizard::Event::Finish => Outcome::Close,
            wizard::Event::Cancel => Outcome::Cancel {
                discovery: self.step == Step::Search,
                pairing: (self.step == Step::Pairing)
                    .then(|| self.target_path().map(str::to_string))
                    .flatten(),
            },
        }
    }

    pub fn list_press(
        &mut self,
        at: Click,
        text: &TextRenderer,
        forms: &Theme,
        cm: &ControlMetrics,
    ) -> Outcome {
        if self.step != Step::Search {
            return Outcome::None;
        }
        match self.list.press(self.list_rect(), at, cm) {
            Press::None => Outcome::None,
            Press::Redraw => Outcome::Redraw,
            Press::Row {
                changed, activate, ..
            } => {
                self.sync_search();
                if activate {
                    self.next(text, forms)
                } else if changed {
                    Outcome::Redraw
                } else {
                    Outcome::None
                }
            }
        }
    }

    pub fn list_key(&mut self, key: &form::Key, cm: &ControlMetrics) -> bool {
        if self.step != Step::Search || !self.list.key(key, self.list_rect(), cm) {
            return false;
        }
        self.sync_search();
        true
    }

    pub fn access(&self, tree: &mut Tree, ctx: &Ctx) {
        let px = self.page.form.units().px(ctx.base_px());
        let mut focus = self.page.access(tree, ROOT, &ctx.text, px);
        if self.step == Step::Search {
            let lay = tilelist::layout(&self.list, self.list_rect(), &ctx.theme.controls());
            focus = Some(tilelist::access(
                &self.list,
                &lay,
                |i| {
                    (
                        self.found[i].alias.clone(),
                        self.found[i].kind.name().to_string(),
                    )
                },
                tree,
                ROOT,
                "found",
            ));
        }
        if let Some(f) = focus {
            tree.set_focus(f);
        }
    }

    pub fn draw(&self, c: &mut Canvas, ctx: &Ctx, icons: &Icons) {
        let cs = &ctx.theme.colors;
        let forms = ctx.forms();
        wizard::draw(
            c,
            &ctx.text,
            &Paint {
                s: 1,
                ..forms.paint()
            },
            &self.page,
            true,
            forms.fonts.size_px,
        );
        if self.step != Step::Search {
            return;
        }
        let px = ctx.theme.fonts.size_px;
        let lay = tilelist::layout(&self.list, self.list_rect(), &ctx.theme.controls());
        tilelist::draw(
            c,
            &ctx.text,
            &Paint {
                s: 1,
                ..ctx.theme.paint()
            },
            &self.list,
            &lay,
            px,
            |c, r, i, sel| {
                let d = &self.found[i];
                let icon = icons.tile(d.kind, lay.text);
                let tile = Tile {
                    icon: Some(&icon),
                    title: &d.alias,
                    tag: "",
                    caption: d.kind.name(),
                    greyed: false,
                };
                tilelist::draw_tile(c, &ctx.text, cs, r, &tile, sel, px, lay.text);
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bz::{Adapter, Answer, Kind};
    use reprisekit::theme::{FontConfig, Metrics, Theme};

    struct T {
        text: TextRenderer,
        forms: Theme,
    }

    fn t() -> T {
        T {
            text: TextRenderer::load(&FontConfig::default()),
            forms: Theme::default(),
        }
    }

    fn dev(path: &str, name: Option<&str>, paired: bool, rssi: Option<i16>) -> Device {
        Device {
            path: path.into(),
            name: name.map(Into::into),
            alias: name.unwrap_or("AA-BB").into(),
            kind: Kind::Gamepad,
            paired,
            rssi,
            ..Device::default()
        }
    }

    fn snap(powered: bool, discovering: bool) -> Snapshot {
        Snapshot {
            adapter: Some(Adapter {
                path: "/a".into(),
                powered,
                discovering,
                ..Adapter::default()
            }),
            devices: vec![
                dev("/a/pad", Some("Pad"), false, Some(-50)),
                dev("/a/cans", Some("Cans"), true, Some(-40)),
                dev("/a/anon", None, false, Some(-30)),
                dev("/a/old", Some("Old"), false, None),
            ],
        }
    }

    fn at_search(t: &T) -> AddWizard {
        let mut w = AddWizard::new(&t.text, &t.forms);
        w.page.form.set_check(READY, true);
        w.event(form::Event::Changed(READY), &t.text, &t.forms);
        assert_eq!(
            w.event(form::Event::Pressed(wizard::NEXT), &t.text, &t.forms),
            Outcome::StartDiscovery
        );
        w.refresh(&snap(true, true), &t.text, &t.forms);
        w
    }

    fn pick_first(w: &mut AddWizard, t: &T) {
        let lay = tilelist::layout(&w.list, LIST, &ControlMetrics::CLASSIC);
        let r = lay.slots[0].rect;
        w.list_press(
            Metrics::default().click(r.x + 20, r.y + 20, 0),
            &t.text,
            &t.forms,
            &ControlMetrics::CLASSIC,
        );
    }

    /// Every page fits its text under every interface and text size: Welcome, the search
    /// running and done, connecting, each pairing request, and each way it finishes.
    #[test]
    fn every_page_fits_its_text() {
        let asks = [
            Ask::PinCode,
            Ask::Passkey,
            Ask::ShowPin("0000".into()),
            Ask::ShowPasskey(123_456),
            Ask::Confirm(123_456),
            Ask::Authorize,
        ];
        let over = reprisekit::interfaces::form_overflows(|_, forms, text| {
            let mut pages = Vec::new();
            let mut w = AddWizard::new(text, forms);
            pages.push(("welcome".to_string(), w.page.form.clone()));
            w.page.form.set_check(READY, true);
            w.event(form::Event::Changed(READY), text, forms);
            w.event(form::Event::Pressed(wizard::NEXT), text, forms);
            w.refresh(&snap(true, true), text, forms);
            pages.push(("searching".into(), w.page.form.clone()));
            w.refresh(&snap(true, false), text, forms);
            pages.push(("searched".into(), w.page.form.clone()));
            w.list.select_row(Some(0));
            w.sync_search();
            w.event(form::Event::Default, text, forms);
            pages.push(("connecting".into(), w.page.form.clone()));
            for ask in &asks {
                let req = AgentRequest {
                    device: "/a/pad".into(),
                    ask: ask.clone(),
                    done: None,
                };
                w.agent(req, text, forms).unwrap();
                pages.push((format!("{ask:?}"), w.page.form.clone()));
            }
            let finished = [
                None,
                Some(Ok("Host is down".to_string())),
                Some(Err("Authentication Failed".to_string())),
            ];
            for how in finished {
                let mut w = AddWizard::new(text, forms);
                w.target = Some(dev("/a/pad", Some("Pad"), false, Some(-50)));
                w.step = Step::Pairing;
                match &how {
                    None => w.paired("/a/pad", None, text, forms),
                    Some(Ok(e)) => w.paired("/a/pad", Some(e.clone()), text, forms),
                    Some(Err(e)) => w.pair_failed("/a/pad", e.clone(), text, forms),
                }
                pages.push((format!("finish {how:?}"), w.page.form.clone()));
            }
            pages
        });
        assert!(over.is_empty(), "{}", over.join("\n"));
    }

    #[test]
    fn welcome_next_waits_for_the_check() {
        let t = t();
        let mut w = AddWizard::new(&t.text, &t.forms);
        assert_eq!(
            w.event(form::Event::Default, &t.text, &t.forms),
            Outcome::None,
            "Next is greyed"
        );
        w.page.form.set_check(READY, true);
        w.event(form::Event::Changed(READY), &t.text, &t.forms);
        assert_eq!(
            w.event(form::Event::Default, &t.text, &t.forms),
            Outcome::StartDiscovery
        );
        assert_eq!(w.step, Step::Search);
        assert_eq!(
            w.event(form::Event::Pressed(wizard::BACK), &t.text, &t.forms),
            Outcome::StopDiscovery
        );
        assert_eq!(w.step, Step::Welcome);
    }

    #[test]
    fn search_lists_named_unpaired_devices_in_range() {
        let t = t();
        let w = at_search(&t);
        let names: Vec<&str> = w.found.iter().map(|d| d.alias.as_str()).collect();
        assert_eq!(
            names,
            ["Pad"],
            "paired, nameless and out-of-range devices are left out"
        );
        assert_eq!(w.status(), "Searching for Bluetooth devices...");
    }

    #[test]
    fn picking_a_device_pairs_it() {
        let t = t();
        let mut w = at_search(&t);
        assert_eq!(
            w.event(form::Event::Default, &t.text, &t.forms),
            Outcome::None,
            "nothing picked"
        );
        pick_first(&mut w, &t);
        assert_eq!(
            w.event(form::Event::Default, &t.text, &t.forms),
            Outcome::Pair("/a/pad".into())
        );
        assert_eq!(w.step, Step::Pairing);
        assert_eq!(w.target_path(), Some("/a/pad"));
    }

    #[test]
    fn the_targets_requests_become_the_page_and_others_come_back() {
        let t = t();
        let mut w = at_search(&t);
        pick_first(&mut w, &t);
        w.event(form::Event::Default, &t.text, &t.forms);
        let (done, wait) = async_channel::bounded(1);
        let other = AgentRequest {
            device: "/a/else".into(),
            ask: Ask::Authorize,
            done: None,
        };
        assert!(w.agent(other, &t.text, &t.forms).is_err());
        let mine = AgentRequest {
            device: "/a/pad".into(),
            ask: Ask::Confirm(1),
            done: Some(done),
        };
        assert!(w.agent(mine, &t.text, &t.forms).is_ok());
        assert_eq!(
            w.event(form::Event::Default, &t.text, &t.forms),
            Outcome::Redraw
        );
        assert_eq!(wait.try_recv(), Ok(Answer::Accept));
        assert!(w.ask.is_none(), "back to Connecting");
    }

    #[test]
    fn a_passkey_request_takes_typed_digits() {
        let t = t();
        let mut w = at_search(&t);
        pick_first(&mut w, &t);
        w.event(form::Event::Default, &t.text, &t.forms);
        let (done, wait) = async_channel::bounded(1);
        let req = AgentRequest {
            device: "/a/pad".into(),
            ask: Ask::Passkey,
            done: Some(done),
        };
        assert!(w.agent(req, &t.text, &t.forms).is_ok());
        for ch in "123456".chars() {
            let ev = w.page.form.key(form::Key::Char(ch));
            w.event(ev, &t.text, &t.forms);
        }
        assert_eq!(
            crate::props::edit_text(&w.page.form, passkey::CODE),
            "123456"
        );
        w.event(form::Event::Default, &t.text, &t.forms);
        assert_eq!(wait.try_recv(), Ok(Answer::Passkey(123456)));
    }

    #[test]
    fn a_confirm_request_keeps_alt_n_for_next() {
        let t = t();
        let mut w = at_search(&t);
        pick_first(&mut w, &t);
        w.event(form::Event::Default, &t.text, &t.forms);
        let (done, _wait) = async_channel::bounded(1);
        let req = AgentRequest {
            device: "/a/pad".into(),
            ask: Ask::Confirm(7),
            done: Some(done),
        };
        assert!(w.agent(req, &t.text, &t.forms).is_ok());
        assert_eq!(
            w.page.form.key(form::Key::Access('d')),
            form::Event::Changed(passkey::NO)
        );
        assert_eq!(
            w.page.form.key(form::Key::Access('n')),
            form::Event::Pressed(wizard::NEXT)
        );
    }

    #[test]
    fn bluez_cancel_takes_only_a_request_it_waits_on() {
        let t = t();
        let mut w = at_search(&t);
        pick_first(&mut w, &t);
        w.event(form::Event::Default, &t.text, &t.forms);
        let shown = AgentRequest {
            device: "/a/pad".into(),
            ask: Ask::ShowPasskey(123456),
            done: None,
        };
        assert!(w.agent(shown, &t.text, &t.forms).is_ok());
        assert!(
            !w.agent_cancel(&t.text, &t.forms),
            "a shown code is not claimed"
        );
        assert!(w.ask.is_some(), "and stays up");
        let (done, _wait) = async_channel::bounded(1);
        let waiting = AgentRequest {
            device: "/a/pad".into(),
            ask: Ask::Passkey,
            done: Some(done),
        };
        assert!(w.agent(waiting, &t.text, &t.forms).is_ok());
        assert!(w.agent_cancel(&t.text, &t.forms));
        assert!(w.ask.is_none());
    }

    #[test]
    fn paired_goes_to_finish_and_a_failure_offers_back() {
        let t = t();
        let mut w = at_search(&t);
        pick_first(&mut w, &t);
        w.event(form::Event::Default, &t.text, &t.forms);
        w.pair_failed(
            "/a/pad",
            "The device did not accept the pairing.".into(),
            &t.text,
            &t.forms,
        );
        assert_eq!(w.step, Step::Finish);
        assert_eq!(
            w.event(form::Event::Pressed(wizard::BACK), &t.text, &t.forms),
            Outcome::StartDiscovery
        );
        assert_eq!(w.step, Step::Search);
        w.refresh(&snap(true, true), &t.text, &t.forms);
        pick_first(&mut w, &t);
        w.event(form::Event::Default, &t.text, &t.forms);
        w.paired("/a/pad", None, &t.text, &t.forms);
        assert_eq!(w.step, Step::Finish);
        assert_eq!(
            w.event(form::Event::Default, &t.text, &t.forms),
            Outcome::Close
        );
    }

    #[test]
    fn radio_off_fails_the_search() {
        let t = t();
        let mut w = at_search(&t);
        w.refresh(&snap(false, false), &t.text, &t.forms);
        assert_eq!(w.step, Step::Finish);
        assert_eq!(w.reason(), Some("Bluetooth was turned off."));
        let mut w = at_search(&t);
        w.refresh(&Snapshot::default(), &t.text, &t.forms);
        assert_eq!(w.reason(), Some("The Bluetooth adapter was removed."));
    }

    #[test]
    fn cancel_stops_what_is_running() {
        let t = t();
        let mut w = at_search(&t);
        assert_eq!(
            w.event(form::Event::Cancel, &t.text, &t.forms),
            Outcome::Cancel {
                discovery: true,
                pairing: None
            }
        );
        let mut w = at_search(&t);
        pick_first(&mut w, &t);
        w.event(form::Event::Default, &t.text, &t.forms);
        assert_eq!(
            w.event(form::Event::Cancel, &t.text, &t.forms),
            Outcome::Cancel {
                discovery: false,
                pairing: Some("/a/pad".into())
            }
        );
    }
}
