//! Welcome's wizard pages: the interface, light or dark with the text size, then the keys worth
//! knowing. The host writes each pick ([`Outcome::Picked`]) and hands back the sample's theme.

use crate::access::{ROOT, Tree};
use crate::classic::widgets::{BUTTON_H, LINE_H};
use crate::controls::combo::COMBO_H;
use crate::controls::form::{self, Control, Item};
use crate::controls::wizard::{self, EXTERIOR_BODY, Style, Wizard};
use crate::interface::Interface;
use crate::look::Paint;
use crate::raster::{Canvas, Rect};
use crate::text::{TextRenderer, wrap};
use crate::theme::Theme;

pub const CLIENT: (i32, i32) = wizard::CLIENT;
/// `[accessibility] text_size`'s steps, in percent.
pub const TEXT_SIZES: [u32; 5] = [100, 125, 150, 175, 200];

const SAMPLE: &str = "sample";
/// The Appearance page's sample at its smallest, centred under the header band.
const SAMPLE_RECT: Rect = Rect::new(73, 66, 350, 188);
const ROW_Y: i32 = 268;
const INTERFACES: [(&str, Interface, &str); 3] = [
    ("classic", Interface::Classic, "Reprise &Classic"),
    ("blue", Interface::Blue, "Reprise Bl&ue"),
    ("glass", Interface::Glass, "Reprise &Glass"),
];
const INTERFACE_W: i32 = 120;
const LIGHT: &str = "light";
const DARK: &str = "dark";
const DARK_X: i32 = SAMPLE_RECT.x + 80;
const TEXT_SIZE: &str = "text_size";
const TEXT_SIZE_W: i32 = 80;
const TEXT_LABEL_W: i32 = 62;
const KEYS: &str = "keys";
const LINK_W: i32 = 170;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Interface,
    Look,
    Ready,
}

/// What the pages show picked: the config's values when Welcome opened, then the user's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Picks {
    pub interface: Interface,
    pub dark: bool,
    /// One of [`TEXT_SIZES`].
    pub text_size: u32,
}

/// Whether `i` comes in dark colours: Blue's schemes are all light.
pub fn dark_offered(i: Interface) -> bool {
    i != Interface::Blue
}

/// A program the last page offers as a button; `id` comes back in [`Outcome::Open`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Link {
    pub id: &'static str,
    pub label: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    None,
    Redraw,
    /// [`Welcome::picks`] moved: write them and [`Welcome::set_sample`] the new look.
    Picked,
    Open(&'static str),
    /// Finish or Skip.
    Close,
}

pub struct Welcome {
    pub page: Page,
    pub wizard: Wizard,
    pub picks: Picks,
    sample: Theme,
    links: Vec<Link>,
}

impl Welcome {
    /// The first page with `picks`, `sample` the look they give; `links` are the last page's
    /// buttons, top to bottom.
    pub fn new(
        picks: Picks,
        sample: Theme,
        links: Vec<Link>,
        text: &TextRenderer,
        forms: &Theme,
    ) -> Welcome {
        let mut w = Welcome {
            page: Page::Interface,
            wizard: Wizard::new(
                Style::Exterior {
                    title: String::new(),
                },
                vec![],
                true,
                false,
            ),
            picks,
            sample,
            links,
        };
        w.go(Page::Interface, text, forms);
        w
    }

    /// Show page `page`, its controls laid out for `forms`' text size.
    pub fn go(&mut self, page: Page, text: &TextRenderer, forms: &Theme) {
        self.page = page;
        let sample = || {
            Item::new(
                SAMPLE,
                SAMPLE_RECT,
                Control::Sample {
                    themes: false,
                    theme: self.sample.clone(),
                    icon: None,
                },
            )
        };
        let (style, items) = match page {
            Page::Interface => {
                let mut items = vec![sample()];
                for (i, (id, interface, label)) in INTERFACES.into_iter().enumerate() {
                    let x = SAMPLE_RECT.x + i as i32 * INTERFACE_W;
                    items.push(
                        Item::radio(id, x, ROW_Y, label, "interface")
                            .checked(self.picks.interface == interface),
                    );
                }
                let style = Style::Interior {
                    title: "Welcome to Reprise".into(),
                    subtitle: "Pick an interface. Settings > Appearance changes it later.".into(),
                    icon: None,
                };
                (style, items)
            }
            Page::Look => {
                let offered = dark_offered(self.picks.interface);
                let dark = offered && self.picks.dark;
                let sizes: Vec<String> = TEXT_SIZES.iter().map(|p| format!("{p}%")).collect();
                let refs: Vec<&str> = sizes.iter().map(String::as_str).collect();
                let combo_x = SAMPLE_RECT.right() - TEXT_SIZE_W;
                let items = vec![
                    sample(),
                    Item::radio(LIGHT, SAMPLE_RECT.x, ROW_Y, "&Light", "mode").checked(!dark),
                    Item::radio(DARK, DARK_X, ROW_Y, "&Dark", "mode")
                        .checked(dark)
                        .enabled(offered),
                    Item::label_at(
                        "text_size_label",
                        combo_x - TEXT_LABEL_W - 4,
                        ROW_Y,
                        TEXT_LABEL_W,
                        "&Text size:",
                    ),
                    Item::combo(
                        TEXT_SIZE,
                        combo_x,
                        ROW_Y - (COMBO_H - LINE_H) / 2 - 2,
                        TEXT_SIZE_W,
                        &refs,
                        refs.len(),
                    ),
                ];
                let style = Style::Interior {
                    title: "Light or dark".into(),
                    subtitle: "And how large Reprise draws its text.".into(),
                    icon: None,
                };
                (style, items)
            }
            Page::Ready => (
                Style::Exterior {
                    title: "Reprise is ready".into(),
                },
                self.ready_items(text, forms),
            ),
        };
        let mut wizard = Wizard::new(style, items, page == Page::Interface, page == Page::Ready);
        if let Some(Control::Button { label, .. }) =
            wizard.form.item_mut(wizard::CANCEL).map(|i| &mut i.control)
        {
            *label = "S&kip".into();
        }
        wizard.set_cancel_enabled(page != Page::Ready);
        let size = TEXT_SIZES.iter().position(|&p| p == self.picks.text_size);
        wizard.form.set_combo(TEXT_SIZE, size.unwrap_or(0));
        if page == Page::Interface {
            let picked = INTERFACES.iter().find(|i| i.1 == self.picks.interface);
            wizard.form.focus_id(picked.map_or(wizard::NEXT, |i| i.0));
        }
        wizard.form.drawn_with(&forms.paint());
        self.wizard = wizard;
    }

    fn ready_items(&self, text: &TextRenderer, forms: &Theme) -> Vec<Item> {
        let body = EXTERIOR_BODY;
        let px = forms.fonts.size_px;
        let paragraphs = [
            "Press Super for the Start menu, or hold it a second to see every shortcut.",
            "Super+Space opens the launcher: type a few letters of a program and press Enter.",
            "Super+V brings back what you copied.",
            "Everything you picked here, and much more, is in Settings.",
        ];
        let mut lines = Vec::new();
        for p in paragraphs {
            if !lines.is_empty() {
                lines.push(String::new());
            }
            lines.extend(wrap(text, p, forms.text.len(body.width), px));
        }
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        let mut items = vec![Item::lines_at(KEYS, body.x, body.y, body.width, &refs)];
        let mut y = body.y + LINE_H * lines.len() as i32 + 16;
        for link in &self.links {
            items.push(Item::button_at(
                link.id, body.x, y, LINK_W, link.label, true,
            ));
            y += BUTTON_H + 6;
        }
        items
    }

    /// The look the samples show, after the host wrote the picks.
    pub fn set_sample(&mut self, theme: Theme) {
        self.sample = theme;
        if let Some(Control::Sample { theme: t, .. }) =
            self.wizard.form.item_mut(SAMPLE).map(|i| &mut i.control)
        {
            *t = self.sample.clone();
        }
    }

    /// Lay the page out again when `forms`' text size is not the one it was built for.
    pub fn follow(&mut self, text: &TextRenderer, forms: &Theme) {
        if self.wizard.form.units() != forms.text {
            let focus = self
                .wizard
                .form
                .focus()
                .and_then(|f| self.wizard.form.items.get(f));
            let focus = focus.map(|i| i.id);
            self.go(self.page, text, forms);
            if let Some(id) = focus {
                self.wizard.form.focus_id(id);
            }
        }
    }

    pub fn event(&mut self, ev: form::Event, text: &TextRenderer, forms: &Theme) -> Outcome {
        match self.wizard.event(ev) {
            wizard::Event::None => Outcome::None,
            wizard::Event::Redraw => Outcome::Redraw,
            wizard::Event::Back => {
                let back = match self.page {
                    Page::Ready => Page::Look,
                    _ => Page::Interface,
                };
                self.go(back, text, forms);
                Outcome::Redraw
            }
            wizard::Event::Next => {
                let next = match self.page {
                    Page::Interface => Page::Look,
                    _ => Page::Ready,
                };
                self.go(next, text, forms);
                Outcome::Redraw
            }
            wizard::Event::Finish | wizard::Event::Cancel => Outcome::Close,
            wizard::Event::Page(form::Event::Changed(id)) => {
                let form = &self.wizard.form;
                let mut picks = self.picks;
                if let Some((_, i, _)) = INTERFACES.iter().find(|i| i.0 == id) {
                    picks.interface = *i;
                    picks.dark &= dark_offered(*i);
                } else if id == LIGHT || id == DARK {
                    picks.dark = form.checked(DARK).unwrap_or(false);
                } else if id == TEXT_SIZE {
                    let at = form.combo_selected(TEXT_SIZE).unwrap_or(0);
                    picks.text_size = TEXT_SIZES[at.min(TEXT_SIZES.len() - 1)];
                }
                if picks == self.picks {
                    return Outcome::Redraw;
                }
                self.picks = picks;
                Outcome::Picked
            }
            wizard::Event::Page(form::Event::Pressed(id)) => {
                match self.links.iter().find(|l| l.id == id) {
                    Some(l) => Outcome::Open(l.id),
                    None => Outcome::Redraw,
                }
            }
            wizard::Event::Page(_) => Outcome::Redraw,
        }
    }

    /// The page's `access` part; the focused node becomes the tree's focus.
    pub fn access(&self, tree: &mut Tree, text: &TextRenderer, px: u32) {
        if let Some(f) = self.wizard.access(tree, ROOT, text, px) {
            tree.set_focus(f);
        }
    }
}

pub fn draw(c: &mut Canvas, text: &TextRenderer, p: &Paint, w: &Welcome, cues: bool, px: u32) {
    wizard::draw(c, text, p, &w.wizard, cues, px);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interfaces;
    use crate::theme::FontConfig;

    fn picks() -> Picks {
        Picks {
            interface: Interface::Classic,
            dark: false,
            text_size: 100,
        }
    }

    fn links() -> Vec<Link> {
        vec![
            Link {
                id: "settings",
                label: "Open &Settings",
            },
            Link {
                id: "import",
                label: "&Import from Another Desktop",
            },
        ]
    }

    fn welcome(theme: &Theme) -> (Welcome, TextRenderer) {
        let text = TextRenderer::load(&theme.fonts);
        let w = Welcome::new(picks(), theme.clone(), links(), &text, theme);
        (w, text)
    }

    #[test]
    fn next_walks_three_pages_and_finish_closes() {
        let theme = Theme::default();
        let (mut w, text) = welcome(&theme);
        assert_eq!(w.page, Page::Interface);
        let next = form::Event::Pressed(wizard::NEXT);
        assert_eq!(w.event(next, &text, &theme), Outcome::Redraw);
        assert_eq!(w.page, Page::Look);
        w.event(next, &text, &theme);
        assert_eq!(w.page, Page::Ready);
        assert_eq!(w.event(next, &text, &theme), Outcome::Close, "Finish");
        w.event(form::Event::Pressed(wizard::BACK), &text, &theme);
        assert_eq!(w.page, Page::Look);
    }

    #[test]
    fn skip_closes_and_is_greyed_on_the_last_page() {
        let theme = Theme::default();
        let (mut w, text) = welcome(&theme);
        assert_eq!(
            w.event(form::Event::Pressed(wizard::CANCEL), &text, &theme),
            Outcome::Close
        );
        w.go(Page::Ready, &text, &theme);
        assert!(matches!(
            w.wizard.form.item(wizard::CANCEL).unwrap().control,
            Control::Button { enabled: false, ref label, .. } if label == "S&kip"
        ));
    }

    #[test]
    fn a_radio_or_the_combo_moves_the_picks() {
        let theme = Theme::default();
        let (mut w, text) = welcome(&theme);
        w.wizard.form.set_check("classic", false);
        w.wizard.form.set_check("glass", true);
        let out = w.event(form::Event::Changed("glass"), &text, &theme);
        assert_eq!(out, Outcome::Picked);
        assert_eq!(w.picks.interface, Interface::Glass);
        w.go(Page::Look, &text, &theme);
        w.wizard.form.set_check(LIGHT, false);
        w.wizard.form.set_check(DARK, true);
        assert_eq!(
            w.event(form::Event::Changed(DARK), &text, &theme),
            Outcome::Picked
        );
        assert!(w.picks.dark);
        w.wizard.form.set_combo(TEXT_SIZE, 2);
        w.event(form::Event::Changed(TEXT_SIZE), &text, &theme);
        assert_eq!(w.picks.text_size, 150);
    }

    #[test]
    fn blue_greys_dark_and_drops_it() {
        let theme = Theme::default();
        let (mut w, text) = welcome(&theme);
        w.picks.dark = true;
        w.event(form::Event::Changed("blue"), &text, &theme);
        assert_eq!(
            (w.picks.interface, w.picks.dark),
            (Interface::Blue, false),
            "Blue is light only"
        );
        w.go(Page::Look, &text, &theme);
        assert!(matches!(
            w.wizard.form.item(DARK).unwrap().control,
            Control::Radio {
                enabled: false,
                checked: false,
                ..
            }
        ));
        assert_eq!(w.wizard.form.checked(LIGHT), Some(true));
    }

    #[test]
    fn a_link_button_opens_its_program() {
        let theme = Theme::default();
        let (mut w, text) = welcome(&theme);
        w.go(Page::Ready, &text, &theme);
        assert_eq!(
            w.event(form::Event::Pressed("settings"), &text, &theme),
            Outcome::Open("settings")
        );
    }

    #[test]
    fn every_page_fits_its_text_under_every_interface() {
        let over = interfaces::form_overflows(|_, theme, text| {
            let mut w = Welcome::new(picks(), theme.clone(), links(), text, theme);
            let mut out = Vec::new();
            for page in [Page::Interface, Page::Look, Page::Ready] {
                w.go(page, text, theme);
                out.push((format!("{page:?}"), w.wizard.form.clone()));
            }
            out
        });
        assert!(over.is_empty(), "{}", over.join("\n"));
    }

    fn fnv(c: &Canvas) -> u64 {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        for v in [c.width, c.height]
            .into_iter()
            .chain(c.pixels.iter().copied())
        {
            for b in v.to_le_bytes() {
                h = (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3);
            }
        }
        h
    }

    #[test]
    fn welcome_pages_pin_under_each_interface_at_scale_1_and_2() {
        crate::pins::fonts();
        let mut got = Vec::new();
        for (name, i) in [
            ("Classic", Interface::Classic),
            ("Blue", Interface::Blue),
            ("Glass", Interface::Glass),
        ] {
            for s in [1, 2] {
                let theme = interfaces::theme_for(i).scaled(s);
                let text = TextRenderer::load(&theme.fonts);
                let picks = Picks {
                    interface: i,
                    ..picks()
                };
                let mut w = Welcome::new(picks, interfaces::theme_for(i), links(), &text, &theme);
                for page in [Page::Interface, Page::Look, Page::Ready] {
                    w.go(page, &text, &theme);
                    let mut c = Canvas::new((CLIENT.0 * s) as u32, (CLIENT.1 * s) as u32);
                    draw(&mut c, &text, &theme.paint(), &w, true, theme.fonts.size_px);
                    got.push((format!("{name} {page:?} {s}"), fnv(&c)));
                }
            }
        }
        crate::pins::check("welcome", &got);
    }

    #[test]
    fn a_new_text_size_lays_the_page_out_again() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let mut w = Welcome::new(picks(), theme.clone(), links(), &text, &theme);
        let grown = theme.with_form_text(150);
        w.follow(&text, &grown);
        assert_eq!(w.wizard.form.units(), grown.text);
        assert_eq!(w.page, Page::Interface);
    }
}
