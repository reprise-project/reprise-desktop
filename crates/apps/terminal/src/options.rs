//! The Options property sheet (General, Appearance, Profiles) and the form that edits a profile.

use reprisekit::controls::form::{self, Control, Form, Item};
use reprisekit::controls::propsheet::{self, PropSheet};
use reprisekit::controls::updown::UpDown;
use reprisekit::raster::Rect;
use reprisekit::trackbar::{self, TrackBar};

use crate::schemes;
use crate::settings::{Cursor, Profile, SCROLLBACK_MAX, Settings, TabPosition};

pub const CLIENT: (i32, i32) = (398, 317);
pub const PROFILE_CLIENT: (i32, i32) = (300, 112);

const TABS: [&str; 3] = ["General", "Appearance", "Profiles"];
const GENERAL: usize = 0;
const APPEARANCE: usize = 1;
const PROFILES: usize = 2;

const SCROLLBACK: (i32, i32) = (0, SCROLLBACK_MAX as i32);
/// The tab position combo's rows, in [`TabPosition`] order.
const TAB_POSITIONS: [(&str, TabPosition); 2] = [
    ("Below the toolbar", TabPosition::Below),
    ("In the title bar", TabPosition::Caption),
];
const SIZE_PX: (i32, i32) = (6, 48);
/// The trackbar's first position is this opacity, in percent.
const OPACITY_MIN: usize = 10;
const OPACITY_STEPS: usize = 91;

/// What the host does after an event.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    None,
    Redraw,
    Apply(Settings),
    ApplyAndClose(Settings),
    Close,
}

pub struct Options {
    pub sheet: PropSheet,
    /// The profile form, open while a profile is added or edited.
    pub editing: Option<ProfileForm>,
    applied: Settings,
    profiles: Vec<Profile>,
}

pub struct ProfileForm {
    pub form: Form,
    /// The profile being edited; none for a new one.
    at: Option<usize>,
}

impl Options {
    pub fn new(s: &Settings) -> Options {
        let pages = vec![
            general_page(s),
            appearance_page(s),
            profiles_page(&s.profiles),
        ];
        let mut o = Options {
            sheet: PropSheet::new(&TABS, pages, CLIENT),
            editing: None,
            applied: s.clone(),
            profiles: s.profiles.clone(),
        };
        o.show_opacity();
        o.show_profiles();
        o.applied = o.settings();
        o.sheet.set_apply_enabled(false);
        o
    }

    /// The settings the sheet's fields hold now.
    pub fn settings(&self) -> Settings {
        let general = &self.sheet.pages[GENERAL];
        let look = &self.sheet.pages[APPEARANCE];
        let cursor = if general.checked("bar") == Some(true) {
            Cursor::Bar
        } else if general.checked("underline") == Some(true) {
            Cursor::Underline
        } else {
            Cursor::Block
        };
        let scheme = look.combo_selected("scheme").unwrap_or(0);
        let tabs = general.combo_selected("tab_position").unwrap_or(0);
        Settings {
            shell: edit_text(general, "shell"),
            tab_position: TAB_POSITIONS[tabs.min(TAB_POSITIONS.len() - 1)].1,
            cursor,
            blink: general.checked("blink").unwrap_or(true),
            scrollback: number(general, "scrollback") as usize,
            confirm_multiline_paste: general.checked("confirm_paste").unwrap_or(true),
            font: edit_text(look, "font"),
            size_px: number(look, "size") as u32,
            scheme: schemes::NAMES[scheme.min(schemes::NAMES.len() - 1)]
                .0
                .into(),
            see_through: look.checked("see_through").unwrap_or(false),
            opacity: (look.track_pos("opacity").unwrap_or(70) + OPACITY_MIN) as u8,
            profiles: self.profiles.clone(),
        }
    }

    pub fn dirty(&self) -> bool {
        self.settings() != self.applied
    }

    pub fn event(&mut self, ev: propsheet::Event) -> Outcome {
        match ev {
            propsheet::Event::None => Outcome::None,
            propsheet::Event::Redraw => Outcome::Redraw,
            propsheet::Event::Page(_, ev) => {
                self.page_event(ev);
                self.sheet.set_apply_enabled(self.dirty());
                Outcome::Redraw
            }
            propsheet::Event::Apply if self.dirty() => Outcome::Apply(self.baseline()),
            propsheet::Event::Apply => Outcome::Redraw,
            propsheet::Event::Ok if self.dirty() => Outcome::ApplyAndClose(self.baseline()),
            propsheet::Event::Ok | propsheet::Event::Cancel => Outcome::Close,
        }
    }

    /// The fields now are the saved settings: Apply greys until the next change.
    fn baseline(&mut self) -> Settings {
        let s = self.settings();
        self.applied = s.clone();
        self.sheet.set_apply_enabled(false);
        s
    }

    fn page_event(&mut self, ev: form::Event) {
        match ev {
            form::Event::Changed("see_through" | "opacity") => self.show_opacity(),
            form::Event::Changed("profiles") => self.show_profiles(),
            form::Event::Pressed("add") => {
                self.editing = Some(ProfileForm::new(None, &Profile::default()));
            }
            form::Event::Pressed("edit") => {
                if let Some(i) = self.picked() {
                    self.editing = Some(ProfileForm::new(Some(i), &self.profiles[i]));
                }
            }
            form::Event::Pressed("remove") => {
                if let Some(i) = self.picked() {
                    self.profiles.remove(i);
                    self.show_profiles();
                }
            }
            _ => {}
        }
    }

    /// The profile form was acted on; a finished one is closed.
    pub fn profile_event(&mut self, ev: form::Event) {
        let Some(editing) = &mut self.editing else {
            return;
        };
        match ev {
            form::Event::Pressed("ok") | form::Event::Default if editing.valid() => {
                let profile = editing.profile();
                match editing.at {
                    Some(i) => self.profiles[i] = profile,
                    None => self.profiles.push(profile),
                }
                self.editing = None;
                self.show_profiles();
                self.sheet.set_apply_enabled(self.dirty());
            }
            form::Event::Pressed("cancel") | form::Event::Cancel => self.editing = None,
            _ => editing.refresh(),
        }
    }

    fn picked(&self) -> Option<usize> {
        match self.sheet.pages[PROFILES]
            .item("profiles")
            .map(|i| &i.control)
        {
            Some(Control::ListBox { list, .. }) => list.selected,
            _ => None,
        }
    }

    /// The opacity label follows the trackbar; both grey with the check box.
    fn show_opacity(&mut self) {
        let page = &mut self.sheet.pages[APPEARANCE];
        let on = page.checked("see_through").unwrap_or(false);
        let pct = page.track_pos("opacity").unwrap_or(0) + OPACITY_MIN;
        page.set_text("opacity_pct", &format!("{pct}%"));
        for id in ["opacity_label", "opacity", "opacity_pct"] {
            page.set_enabled(id, on);
        }
    }

    /// The list shows `profiles`; Edit and Remove need a pick.
    fn show_profiles(&mut self) {
        let lines: Vec<String> = self.profiles.iter().map(profile_line).collect();
        let page = &mut self.sheet.pages[PROFILES];
        if let Some(Control::ListBox { list, .. }) =
            page.item_mut("profiles").map(|i| &mut i.control)
        {
            list.items = lines;
            list.selected = list.selected.filter(|&i| i < list.items.len());
            list.top = list.top.min(list.items.len().saturating_sub(1));
        }
        let picked = self.picked().is_some();
        let page = &mut self.sheet.pages[PROFILES];
        page.set_enabled("edit", picked);
        page.set_enabled("remove", picked);
    }
}

impl ProfileForm {
    fn new(at: Option<usize>, p: &Profile) -> ProfileForm {
        let mut f = ProfileForm {
            form: Form::new(vec![
                Item::label("name_label", Rect::new(12, 16, 60, 13), "&Name:"),
                Item::edit("name", Rect::new(76, 12, 212, 21), &p.name, true),
                Item::label("command_label", Rect::new(12, 46, 60, 13), "&Command:"),
                Item::edit("command", Rect::new(76, 42, 212, 21), &p.command, true),
                Item::button("ok", Rect::new(132, 80, 75, 23), "OK", false, true),
                Item::button("cancel", Rect::new(213, 80, 75, 23), "Cancel", true, false),
            ]),
            at,
        };
        f.refresh();
        f
    }

    fn profile(&self) -> Profile {
        Profile {
            name: edit_text(&self.form, "name"),
            command: edit_text(&self.form, "command"),
        }
    }

    fn valid(&self) -> bool {
        let p = self.profile();
        !p.name.is_empty() && !p.command.is_empty()
    }

    /// OK waits for both fields.
    fn refresh(&mut self) {
        let valid = self.valid();
        self.form.set_enabled("ok", valid);
    }
}

fn profile_line(p: &Profile) -> String {
    format!("{}  ({})", p.name, p.command)
}

fn edit_text(page: &Form, id: &str) -> String {
    page.edit_text(id).trim().to_owned()
}

fn number(page: &Form, id: &str) -> i32 {
    match page.item(id).map(|i| &i.control) {
        Some(Control::UpDown(u)) => u.value(),
        _ => 0,
    }
}

fn spin(id: &'static str, rect: Rect, value: i32, (lo, hi): (i32, i32)) -> Item {
    Item::new(
        id,
        rect,
        Control::UpDown(UpDown::new(value.clamp(lo, hi), lo, hi).with_buddy()),
    )
}

fn general_page(s: &Settings) -> Form {
    let positions: Vec<&str> = TAB_POSITIONS.iter().map(|p| p.0).collect();
    let mut tabs = Item::combo("tab_position", 140, 236, 160, &positions, positions.len());
    if let Control::Combo(c) = &mut tabs.control {
        c.selected = TAB_POSITIONS
            .iter()
            .position(|p| p.1 == s.tab_position)
            .unwrap_or(0);
    }
    Form::new(vec![
        Item::label(
            "shell_label",
            Rect::new(16, 40, 366, 13),
            "&Shell (empty runs your login shell):",
        ),
        Item::edit("shell", Rect::new(16, 56, 366, 21), &s.shell, true),
        Item::group("cursor_group", Rect::new(16, 90, 366, 52), "Cursor"),
        Item::radio("block", 30, 110, "&Block", "cursor").checked(s.cursor == Cursor::Block),
        Item::radio("bar", 140, 110, "Ba&r", "cursor").checked(s.cursor == Cursor::Bar),
        Item::radio("underline", 250, 110, "&Underline", "cursor")
            .checked(s.cursor == Cursor::Underline),
        Item::check("blink", 16, 156, "Blin&king cursor").checked(s.blink),
        Item::check("confirm_paste", 200, 156, "Confirm &multi-line pastes")
            .checked(s.confirm_multiline_paste),
        Item::label(
            "scrollback_label",
            Rect::new(16, 186, 200, 13),
            "S&crollback lines:",
        ),
        spin(
            "scrollback",
            Rect::new(16, 202, 90, 21),
            s.scrollback.min(SCROLLBACK.1 as usize) as i32,
            SCROLLBACK,
        ),
        Item::label(
            "tab_position_label",
            Rect::new(16, 240, 120, 13),
            "&Tabs (new windows):",
        ),
        tabs,
    ])
}

fn appearance_page(s: &Settings) -> Form {
    let labels: Vec<&str> = schemes::NAMES.iter().map(|n| n.1).collect();
    let mut items = vec![
        Item::label(
            "font_label",
            Rect::new(16, 40, 250, 13),
            "&Font family (empty picks a monospace face):",
        ),
        Item::edit("font", Rect::new(16, 56, 250, 21), &s.font, true),
        Item::label("size_label", Rect::new(284, 40, 90, 13), "Si&ze in pixels:"),
        spin(
            "size",
            Rect::new(284, 56, 79, 21),
            s.size_px as i32,
            SIZE_PX,
        ),
        Item::label(
            "scheme_label",
            Rect::new(16, 92, 250, 13),
            "Colour &scheme:",
        ),
        Item::combo("scheme", 16, 108, 200, &labels, labels.len()),
        Item::group("background", Rect::new(16, 146, 366, 96), "Background"),
        Item::check("see_through", 30, 168, "S&ee-through under Glass").checked(s.see_through),
        Item::label("opacity_label", Rect::new(30, 200, 56, 13), "&Opacity:"),
        Item::new(
            "opacity",
            Rect::new(88, 192, 230, trackbar::HEIGHT),
            Control::TrackBar(opacity_bar(s.opacity)),
        ),
        Item::label("opacity_pct", Rect::new(326, 200, 44, 13), ""),
    ];
    if let Some(Item {
        control: Control::Combo(c),
        ..
    }) = items.iter_mut().find(|i| i.id == "scheme")
    {
        c.selected = schemes::NAMES
            .iter()
            .position(|n| n.0 == s.scheme)
            .unwrap_or(0);
    }
    Form::new(items)
}

fn opacity_bar(opacity: u8) -> TrackBar {
    let mut bar = TrackBar::new(
        usize::from(opacity).saturating_sub(OPACITY_MIN),
        OPACITY_STEPS,
    );
    bar.page = 10;
    bar.tick_every = 10;
    bar
}

fn profiles_page(profiles: &[Profile]) -> Form {
    let lines = profiles.iter().map(profile_line).collect();
    Form::new(vec![
        Item::label(
            "profiles_label",
            Rect::new(16, 40, 366, 13),
            "Each profile is an entry in the + menu that runs its command.",
        ),
        Item::new(
            "profiles",
            Rect::new(16, 62, 270, 180),
            Control::ListBox {
                list: reprisekit::controls::listbox::ListBox::new(lines, None),
                icons: Vec::new(),
            },
        ),
        Item::button_at("add", 298, 62, 84, "&Add...", true),
        Item::button_at("edit", 298, 92, 84, "&Edit...", false),
        Item::button_at("remove", 298, 122, 84, "&Remove", false),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use reprisekit::textedit::TextEdit;

    fn typed(form: &mut Form, id: &str, text: &str) {
        if let Some(Control::Edit { edit, .. }) = form.item_mut(id).map(|i| &mut i.control) {
            *edit = TextEdit::new(text, false);
        }
    }

    fn changed(o: &mut Options, page: usize, id: &'static str) -> Outcome {
        o.event(propsheet::Event::Page(page, form::Event::Changed(id)))
    }

    fn pressed(o: &mut Options, id: &'static str) {
        o.event(propsheet::Event::Page(PROFILES, form::Event::Pressed(id)));
    }

    fn pick(o: &mut Options, i: usize) {
        if let Some(Control::ListBox { list, .. }) = o.sheet.pages[PROFILES]
            .item_mut("profiles")
            .map(|i| &mut i.control)
        {
            list.selected = Some(i);
        }
        changed(o, PROFILES, "profiles");
    }

    fn other() -> Settings {
        Settings {
            shell: "/bin/zsh".into(),
            tab_position: TabPosition::Caption,
            cursor: Cursor::Underline,
            blink: false,
            scrollback: 2500,
            confirm_multiline_paste: false,
            font: "Iosevka".into(),
            size_px: 17,
            scheme: "dark".into(),
            see_through: true,
            opacity: 35,
            profiles: vec![Profile {
                name: "Top".into(),
                command: "htop".into(),
            }],
        }
    }

    #[test]
    fn the_fields_read_back_the_settings_they_were_built_from() {
        for s in [Settings::default(), other()] {
            assert_eq!(Options::new(&s).settings(), s);
        }
    }

    #[test]
    fn the_pages_and_the_profile_form_fit_their_text() {
        let over = reprisekit::interfaces::form_overflows(|_, _, _| {
            let o = Options::new(&other());
            let sheet = o.sheet;
            let mut forms: Vec<_> = sheet.tabs.labels.into_iter().zip(sheet.pages).collect();
            forms.push((
                "Profile".into(),
                ProfileForm::new(None, &Profile::default()).form,
            ));
            forms
        });
        assert!(over.is_empty(), "{}", over.join("\n"));
    }

    #[test]
    fn apply_wakes_on_a_change_and_greys_after() {
        let mut o = Options::new(&Settings::default());
        assert_eq!(
            o.event(propsheet::Event::Ok),
            Outcome::Close,
            "nothing changed"
        );
        o.sheet.pages[GENERAL].set_check("blink", false);
        changed(&mut o, GENERAL, "blink");
        assert!(o.dirty());
        let want = Settings {
            blink: false,
            ..Settings::default()
        };
        assert_eq!(
            o.event(propsheet::Event::Apply),
            Outcome::Apply(want.clone())
        );
        assert!(!o.dirty(), "the applied settings are the new baseline");
        o.sheet.pages[APPEARANCE].set_combo("scheme", 1);
        changed(&mut o, APPEARANCE, "scheme");
        assert_eq!(
            o.event(propsheet::Event::Ok),
            Outcome::ApplyAndClose(Settings {
                scheme: "light".into(),
                ..want
            })
        );
        assert_eq!(o.event(propsheet::Event::Cancel), Outcome::Close);
    }

    #[test]
    fn opacity_follows_the_see_through_check_box() {
        let mut o = Options::new(&Settings::default());
        let page = &o.sheet.pages[APPEARANCE];
        assert!(
            matches!(&page.item("opacity").unwrap().control, Control::TrackBar(t) if !t.enabled)
        );
        o.sheet.pages[APPEARANCE].set_check("see_through", true);
        changed(&mut o, APPEARANCE, "see_through");
        o.sheet.pages[APPEARANCE].set_track("opacity", 40);
        changed(&mut o, APPEARANCE, "opacity");
        let page = &o.sheet.pages[APPEARANCE];
        assert!(
            matches!(&page.item("opacity").unwrap().control, Control::TrackBar(t) if t.enabled)
        );
        assert!(
            matches!(&page.item("opacity_pct").unwrap().control, Control::Label { text, .. } if text == "50%")
        );
        assert_eq!(o.settings().opacity, 50);
    }

    #[test]
    fn profiles_are_added_edited_and_removed() {
        let mut o = Options::new(&Settings::default());
        pressed(&mut o, "add");
        let f = o.editing.as_mut().expect("the form opens");
        typed(&mut f.form, "name", "Top");
        o.profile_event(form::Event::Changed("name"));
        let ok = o.editing.as_ref().unwrap().form.item("ok").unwrap();
        assert!(
            matches!(&ok.control, Control::Button { enabled: false, .. }),
            "OK waits for the command"
        );
        o.profile_event(form::Event::Default);
        assert!(o.editing.is_some(), "an incomplete profile is not taken");
        typed(&mut o.editing.as_mut().unwrap().form, "command", "htop");
        o.profile_event(form::Event::Changed("command"));
        o.profile_event(form::Event::Pressed("ok"));
        assert!(o.editing.is_none());
        assert_eq!(
            o.settings().profiles,
            vec![Profile {
                name: "Top".into(),
                command: "htop".into()
            }]
        );
        assert!(o.dirty());

        pick(&mut o, 0);
        pressed(&mut o, "edit");
        typed(&mut o.editing.as_mut().unwrap().form, "command", "btop");
        o.profile_event(form::Event::Changed("command"));
        o.profile_event(form::Event::Pressed("ok"));
        assert_eq!(o.settings().profiles[0].command, "btop");

        pressed(&mut o, "add");
        o.profile_event(form::Event::Pressed("cancel"));
        assert_eq!(o.settings().profiles.len(), 1, "Cancel adds nothing");

        pressed(&mut o, "remove");
        assert!(o.settings().profiles.is_empty());
        assert!(!o.dirty(), "back to what was applied");
    }

    #[test]
    fn edit_and_remove_need_a_pick() {
        let mut o = Options::new(&other());
        let enabled = |o: &Options, id| {
            matches!(
                o.sheet.pages[PROFILES].item(id).unwrap().control,
                Control::Button { enabled: true, .. }
            )
        };
        assert!(!enabled(&o, "edit") && !enabled(&o, "remove"));
        pick(&mut o, 0);
        assert!(enabled(&o, "edit") && enabled(&o, "remove"));
    }

    /// Writes target/terminal-options-preview.png (the three pages side by side) for a visual check.
    #[test]
    #[ignore]
    fn render_sheet() {
        use reprisekit::raster::Canvas;
        use reprisekit::text::TextRenderer;
        use reprisekit::theme::{FontConfig, Theme};
        let theme = Theme {
            access_keys: true,
            ..Theme::default()
        };
        let text = TextRenderer::load(&FontConfig::default());
        let mut o = Options::new(&other());
        let (w, h) = CLIENT;
        let mut out = Canvas::new(3 * w as u32, h as u32);
        for tab in 0..3 {
            o.sheet.tabs.open = tab;
            let mut c = Canvas::new(w as u32, h as u32);
            c.fill_rect(c.bounds(), theme.colors.button_face);
            propsheet::draw(&mut c, &text, &theme.paint(), &o.sheet, false, 11);
            out.blit_canvas(tab as i32 * w, 0, &c);
        }
        reprisekit::raster::write_preview(&out, "terminal-options-preview.png");
    }

    #[test]
    fn a_fresh_sheet_with_an_out_of_range_value_is_clean() {
        let o = Options::new(&Settings {
            size_px: 500,
            scrollback: 9_999_999,
            ..Settings::default()
        });
        assert!(!o.dirty());
    }
}
