use crate::access::{
    self, Tree,
    accesskit::{NodeId, Role},
};
use crate::classic::widgets::{BUTTON_H, BUTTON_W, LINE_H, draw_focus_ring, draw_label};
use crate::controls::form;
use crate::controls::grouplist::{self, GroupList, Row};
use crate::controls::metrics::ControlMetrics;
use crate::controls::scrollbar::ThumbDrag;
use crate::cues::Cues;
use crate::look::Paint;
use crate::look::widgets::{
    draw_button, draw_checkbox, draw_checkbox_disabled, draw_disabled_label, draw_group_box,
};
use crate::pointer::{Axis, Pointer};
use crate::raster::{Canvas, Rect};
use crate::style::Part;
use crate::text::{Align, TextRenderer, split_mnemonic};

const ICON_AT: Rect = Rect::new(11, 11, 32, 32);
const PROMPT_AT: Rect = Rect::new(54, 12, 327, LINE_H);
const FILE_AT: Rect = Rect::new(54, 38, 327, LINE_H);
const FILE_NAME_X: i32 = 90;
const GROUP_R: Rect = Rect::new(9, 66, 368, 288);
const GROUP_LABEL: (i32, i32) = (18, 61);
const CHECK_AT: (i32, i32) = (27, 299);
const BROWSE_R: Rect = Rect::new(291, 320, BUTTON_W, BUTTON_H);
const OK_R: Rect = Rect::new(210, 363, BUTTON_W, BUTTON_H);
const CANCEL_R: Rect = Rect::new(291, 363, BUTTON_W, BUTTON_H);

pub const CLIENT: (i32, i32) = (390, 395);
pub const TITLE: &str = "Open With";
pub const ICON: &str = "system-run";
pub const PROGRAM_ICON: &str = "application-x-executable";
pub const PROMPT: &str = "Choose the program you want to use to open this file:";
pub const RECOMMENDED: &str = "Recommended Programs:";
pub const OTHERS: &str = "Other Programs:";
pub const GROUP: &str = "&Programs";
pub const ALWAYS: &str = "&Always use the selected program to open this kind of file";
pub const BROWSE: &str = "&Browse...";
const FOLDER_ICON: &str = "folder";

pub(crate) const LIST: crate::raster::Rect = crate::raster::Rect::new(24, 81, 338, 210);
pub const ROWS: usize = ((LIST.height - 4) / grouplist::ROW_H) as usize;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Program {
    /// A desktop-file id, or an absolute path for a Browse pick.
    pub id: String,
    pub name: String,
    pub icon: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Check {
    pub on: bool,
    pub enabled: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    List,
    Always,
    Browse,
    Ok,
    Cancel,
}

const TAB_ORDER: [Focus; 5] = [
    Focus::List,
    Focus::Always,
    Focus::Browse,
    Focus::Ok,
    Focus::Cancel,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    List(grouplist::Hit),
    Always,
    Browse,
    Ok,
    Cancel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub client: Rect,
    pub icon: Rect,
    pub prompt: Rect,
    pub file: Rect,
    pub group: Rect,
    pub group_label: Rect,
    pub list: grouplist::Layout,
    pub check: Rect,
    pub check_label: Rect,
    pub browse: Rect,
    pub ok: Rect,
    pub cancel: Rect,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    None,
    Redraw,
    /// The host picks a program file and hands it to `add_program`.
    /// OK with this program picked; `always` is the box.
    Choose {
        id: String,
        always: bool,
    },
    Browse,
    Cancel,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenWith {
    pub file: String,
    pub list: GroupList<Program>,
    pub always: Check,
    pub browse_enabled: bool,
    pub focus: Focus,
    pointer: Pointer<Focus>,
    thumb: ThumbDrag,
}

fn rows(recommended: Vec<Program>, others: Vec<Program>) -> Vec<Row<Program>> {
    let mut rows = Vec::new();
    for (label, programs) in [(RECOMMENDED, recommended), (OTHERS, others)] {
        if programs.is_empty() {
            continue;
        }
        rows.push(Row::Group(label.into()));
        rows.extend(programs.into_iter().map(|p| Row::Item {
            name: p.name.clone(),
            data: p,
        }));
    }
    rows
}

pub fn icon_name(row: &Row<Program>) -> Option<&str> {
    match row {
        Row::Group(_) => Some(FOLDER_ICON),
        Row::Item { data, .. } => data.icon.as_deref(),
    }
}

/// The label's width was set at the device font size, so only its
/// position and height scale.
fn label_rect(r: Rect, s: i32) -> Rect {
    Rect::new(r.x * s, r.y * s, r.width, r.height * s)
}

fn redraw_if(changed: bool) -> Event {
    if changed { Event::Redraw } else { Event::None }
}

impl OpenWith {
    pub fn new(file: &str, recommended: Vec<Program>, others: Vec<Program>) -> OpenWith {
        OpenWith {
            file: file.into(),
            list: GroupList::new(rows(recommended, others)),
            always: Check {
                on: false,
                enabled: true,
            },
            browse_enabled: true,
            focus: Focus::List,
            pointer: Pointer::new(),
            thumb: ThumbDrag::default(),
        }
    }

    pub fn chosen(&self) -> Option<&Program> {
        self.list.chosen()
    }

    pub fn ok_enabled(&self) -> bool {
        self.chosen().is_some()
    }

    /// Replace the programs, keeping the pick when its id is still
    /// listed; otherwise the first program is picked.
    pub fn set_programs(&mut self, recommended: Vec<Program>, others: Vec<Program>) {
        let keep = self.chosen().map(|p| p.id.clone());
        self.list = GroupList::new(rows(recommended, others));
        if let Some(id) = keep {
            self.select_id(&id);
        }
        self.refocus();
    }

    pub fn select_id(&mut self, id: &str) -> bool {
        let at = self
            .list
            .rows
            .iter()
            .position(|r| matches!(r, Row::Item { data, .. } if data.id == id));
        if let Some(i) = at {
            self.list.select(i, ROWS);
        }
        at.is_some()
    }

    pub fn add_program(&mut self, p: Program) {
        self.focus = Focus::List;
        if self.select_id(&p.id) {
            return;
        }
        let rows = &mut self.list.rows;
        if !rows
            .iter()
            .any(|r| matches!(r, Row::Group(g) if g == OTHERS))
        {
            rows.push(Row::Group(OTHERS.into()));
        }
        rows.push(Row::Item {
            name: p.name.clone(),
            data: p,
        });
        let last = rows.len() - 1;
        self.list.select(last, ROWS);
    }

    fn focusable(&self, f: Focus) -> bool {
        match f {
            Focus::List | Focus::Cancel => true,
            Focus::Always => self.always.enabled,
            Focus::Browse => self.browse_enabled,
            Focus::Ok => self.ok_enabled(),
        }
    }

    fn refocus(&mut self) {
        if !self.focusable(self.focus) {
            self.focus = Focus::List;
        }
    }

    fn press(&mut self, f: Focus) -> Event {
        match f {
            Focus::List => Event::None,
            Focus::Always if self.always.enabled => {
                self.always.on = !self.always.on;
                self.focus = Focus::Always;
                Event::Redraw
            }
            Focus::Browse if self.browse_enabled => Event::Browse,
            Focus::Ok => match self.chosen() {
                Some(p) => Event::Choose {
                    id: p.id.clone(),
                    always: self.always.on,
                },
                None => Event::None,
            },
            Focus::Cancel => Event::Cancel,
            Focus::Always | Focus::Browse => Event::None,
        }
    }

    pub fn key(&mut self, key: form::Key) -> Event {
        use form::Key;
        match key {
            Key::Tab { back } => {
                let n = TAB_ORDER.len();
                let mut at = TAB_ORDER.iter().position(|&f| f == self.focus).unwrap_or(0);
                // The list and Cancel always take the focus, so this stops within a lap.
                loop {
                    at = (if back { at + n - 1 } else { at + 1 }) % n;
                    if self.focusable(TAB_ORDER[at]) {
                        break;
                    }
                }
                self.focus = TAB_ORDER[at];
                Event::Redraw
            }
            Key::Escape => Event::Cancel,
            Key::Enter => match self.focus {
                Focus::Cancel => Event::Cancel,
                Focus::Browse => self.press(self.focus),
                _ => self.press(Focus::Ok),
            },
            Key::Space => self.press(self.focus),
            Key::Access(c) => match c.to_ascii_lowercase() {
                'a' => self.press(Focus::Always),
                'b' => self.press(Focus::Browse),
                'p' => {
                    self.focus = Focus::List;
                    Event::Redraw
                }
                _ => Event::None,
            },
            Key::Up | Key::Down | Key::PageUp | Key::PageDown | Key::Home | Key::End
                if self.focus == Focus::List =>
            {
                let delta = match key {
                    Key::Up => -1,
                    Key::Down => 1,
                    Key::PageUp => -(ROWS as i64),
                    Key::PageDown => ROWS as i64,
                    Key::Home => i64::MIN / 2,
                    _ => i64::MAX / 2,
                };
                redraw_if(self.list.step(delta, ROWS))
            }
            // Hosts flatten one-character inserts to `Key::Char` (`form::flat_key`)
            // and drop longer ones, so no raw `Key::Edit` insert lands here.
            Key::Char(c) if self.focus == Focus::List => self.type_ahead(c),
            _ => Event::None,
        }
    }

    fn type_ahead(&mut self, c: char) -> Event {
        if !c.is_alphanumeric() {
            return Event::None;
        }
        redraw_if(self.list.jump(c, ROWS))
    }

    pub fn click(&mut self, hit: Option<Hit>) -> Event {
        let Some(hit) = hit else {
            return Event::None;
        };
        match hit {
            Hit::List(grouplist::Hit::Row(i)) => {
                self.focus = Focus::List;
                if i < self.list.rows.len() {
                    self.list.select(i, ROWS);
                }
                Event::Redraw
            }
            Hit::List(h) => {
                let Some(bar) = h.bar() else {
                    return Event::None;
                };
                let by = self.thumb.press(Axis::Vertical, bar).by(1, ROWS as i64);
                redraw_if(self.list.scroll(by as i32, ROWS))
            }
            Hit::Always => self.arm(Focus::Always),
            Hit::Browse => self.arm(Focus::Browse),
            Hit::Ok => self.arm(Focus::Ok),
            Hit::Cancel => self.arm(Focus::Cancel),
        }
    }

    fn arm(&mut self, f: Focus) -> Event {
        if !self.focusable(f) {
            return Event::None;
        }
        if f != Focus::Ok {
            self.focus = f;
        }
        self.pointer.press(Some(f), false);
        Event::Redraw
    }

    pub fn holding(&self) -> bool {
        self.pointer.holding() || self.thumb.holding()
    }

    pub fn drag(&mut self, lay: &Layout, y: i32) -> Option<Event> {
        let grab = self.thumb.grab(Axis::Vertical)?;
        let top = grouplist::top_at(&self.list, &lay.list, y, grab);
        let moved = top != self.list.top;
        self.list.top = top;
        Some(redraw_if(moved))
    }

    fn button(hit: Option<Hit>) -> Option<Focus> {
        match hit {
            Some(Hit::Always) => Some(Focus::Always),
            Some(Hit::Browse) => Some(Focus::Browse),
            Some(Hit::Ok) => Some(Focus::Ok),
            Some(Hit::Cancel) => Some(Focus::Cancel),
            _ => None,
        }
    }

    pub fn motion(&mut self, hit: Option<Hit>) -> Event {
        if self.pointer.holding() && self.pointer.motion(Self::button(hit)) {
            Event::Redraw
        } else {
            Event::None
        }
    }

    pub fn release(&mut self, hit: Option<Hit>) -> Event {
        if self.thumb.release() {
            return Event::Redraw;
        }
        if !self.pointer.holding() {
            return Event::None;
        }
        self.pointer.motion(Self::button(hit));
        let Some(f) = self.pointer.release() else {
            return Event::Redraw;
        };
        match self.press(f) {
            Event::None => Event::Redraw,
            e => e,
        }
    }

    pub fn double_click(&mut self, hit: Option<Hit>) -> Event {
        let row = matches!(hit, Some(Hit::List(grouplist::Hit::Row(_))));
        let clicked = self.click(hit);
        if !row {
            return clicked;
        }
        match self.press(Focus::Ok) {
            Event::None => clicked,
            chosen => chosen,
        }
    }

    /// `rows` counts down as positive.
    pub fn wheel(&mut self, hit: Option<Hit>, rows: i32) -> Event {
        match hit {
            Some(Hit::List(_)) => redraw_if(self.list.scroll(rows, ROWS)),
            _ => Event::None,
        }
    }

    pub fn layout(
        &self,
        text: &TextRenderer,
        px: u32,
        origin: (i32, i32),
        cm: &ControlMetrics,
    ) -> Layout {
        let at = |r: Rect| Rect::new(r.x + origin.0, r.y + origin.1, r.width, r.height);
        let (plain_group, _) = split_mnemonic(GROUP);
        let (plain_always, _) = split_mnemonic(ALWAYS);
        let check = at(Rect::new(CHECK_AT.0, CHECK_AT.1, cm.check, cm.check));
        Layout {
            client: at(Rect::new(0, 0, CLIENT.0, CLIENT.1)),
            icon: at(ICON_AT),
            prompt: at(PROMPT_AT),
            file: at(FILE_AT),
            group: at(GROUP_R),
            group_label: at(Rect::new(
                GROUP_LABEL.0,
                GROUP_LABEL.1,
                text.measure(&plain_group, false, px),
                LINE_H,
            )),
            list: grouplist::layout(&self.list, at(LIST), cm),
            check,
            check_label: Rect::new(
                check.x + cm.check_label,
                check.y,
                text.measure(&plain_always, false, px),
                cm.check,
            ),
            browse: at(BROWSE_R),
            ok: at(OK_R),
            cancel: at(CANCEL_R),
        }
    }

    pub fn hit(&self, lay: &Layout, x: i32, y: i32) -> Option<Hit> {
        if lay.ok.contains(x, y) {
            return Some(Hit::Ok);
        }
        if lay.cancel.contains(x, y) {
            return Some(Hit::Cancel);
        }
        if lay.browse.contains(x, y) {
            return Some(Hit::Browse);
        }
        if lay.check.contains(x, y) || lay.check_label.contains(x, y) {
            return Some(Hit::Always);
        }
        if lay.list.frame.contains(x, y) {
            return grouplist::hit(&self.list, &lay.list, x, y).map(Hit::List);
        }
        None
    }

    /// `lay` is laid out at the device font size, so its label widths are
    /// already device px.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &self,
        c: &mut Canvas,
        text: &TextRenderer,
        p: &Paint,
        lay: &Layout,
        icon: Option<&Canvas>,
        row_icons: &[Option<&Canvas>],
        cues: Cues,
        px: u32,
    ) {
        let Paint { cs, s, active, .. } = *p;
        let dots = cues.focus && active;
        c.fill_rect(lay.client.scaled(s), cs.button_face);
        if let Some(ic) = icon {
            let r = lay.icon.scaled(s);
            c.blit_over(
                r.x + (r.width - ic.width as i32) / 2,
                r.y + (r.height - ic.height as i32) / 2,
                ic,
            );
        }
        text.draw(
            c,
            lay.prompt.scaled(s),
            PROMPT,
            false,
            cs.button_text,
            Align::Left,
            px,
        );
        text.draw(
            c,
            lay.file.scaled(s),
            "File:",
            false,
            cs.button_text,
            Align::Left,
            px,
        );
        let indent = FILE_NAME_X - FILE_AT.x;
        let name_r = Rect::new(
            lay.file.x + indent,
            lay.file.y,
            lay.file.width - indent,
            lay.file.height,
        );
        let name = text.ellipsize(&self.file, name_r.width * s, false, px);
        text.draw(
            c,
            name_r.scaled(s),
            &name,
            false,
            cs.button_text,
            Align::Left,
            px,
        );
        let label = label_rect(lay.group_label, s);
        draw_group_box(
            c,
            text,
            p,
            lay.group.scaled(s),
            label,
            &cues.label(GROUP),
            px,
        );
        let shown = match (self.focus == Focus::List && active, dots) {
            (false, _) => grouplist::Shown::Away,
            (true, false) => grouplist::Shown::Focused,
            (true, true) => grouplist::Shown::Cued,
        };
        grouplist::draw(c, text, p, &self.list, &lay.list, row_icons, shown, px);
        let (b, lr) = (lay.check.scaled(s), label_rect(lay.check_label, s));
        let always = cues.label(ALWAYS);
        if self.always.enabled {
            draw_checkbox(c, p, b, self.always.on, self.pointer.of(Focus::Always));
            draw_label(
                c,
                text,
                lr,
                &always,
                false,
                cs.button_text,
                Align::Left,
                px,
                s,
            );
        } else {
            draw_checkbox_disabled(c, p, b, self.always.on);
            draw_disabled_label(
                c,
                text,
                p,
                lr,
                &always,
                Align::Left,
                px,
                Some(Part::CheckBox),
            );
        }
        if self.focus == Focus::Always && dots {
            let ring = Rect::new(lr.x - s, lr.y - s, lr.width + 2 * s, lr.height + 2 * s);
            draw_focus_ring(c, ring, s, cs.button_text);
        }
        let ring = |f: Focus| {
            active
                && match self.focus {
                    Focus::Browse | Focus::Cancel => f == self.focus,
                    _ => f == Focus::Ok,
                }
        };
        for (r, label, enabled, f) in [
            (
                lay.browse,
                &*cues.label(BROWSE),
                self.browse_enabled,
                Focus::Browse,
            ),
            (lay.ok, "OK", self.ok_enabled(), Focus::Ok),
            (lay.cancel, "Cancel", true, Focus::Cancel),
        ] {
            draw_button(
                c,
                text,
                p,
                r.scaled(s),
                label,
                ring(f),
                self.pointer.of(f),
                enabled,
                dots && self.focus == f,
                px,
            );
        }
    }

    pub fn access(&self, tree: &mut Tree, lay: &Layout) -> NodeId {
        let root = access::ROOT;
        let words = format!("{PROMPT} File: {}", self.file);
        tree.push(
            root,
            "text",
            access::label(lay.prompt.union(&lay.file), &words, true),
        );
        let group = tree.push(root, "group", access::named(Role::Group, lay.group, GROUP));
        let list = grouplist::access(&self.list, &lay.list, tree, group, "list");
        let always = tree.push(
            root,
            "always",
            access::toggle(
                Role::CheckBox,
                lay.check.union(&lay.check_label),
                ALWAYS,
                self.always.on.into(),
                self.always.enabled,
            ),
        );
        let browse = tree.push(
            root,
            "browse",
            access::button(lay.browse, BROWSE, self.browse_enabled, false),
        );
        let ok = tree.push(
            root,
            "ok",
            access::button(lay.ok, "OK", self.ok_enabled(), true),
        );
        let cancel = tree.push(
            root,
            "cancel",
            access::button(lay.cancel, "Cancel", true, false),
        );
        let focus = match self.focus {
            Focus::List => list,
            Focus::Always => always,
            Focus::Browse => browse,
            Focus::Ok => ok,
            Focus::Cancel => cancel,
        };
        tree.set_focus(focus);
        focus
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::look::classic_paint;
    use crate::pointer::Interaction;
    use crate::theme::ColorScheme;
    use form::Key;

    fn p(id: &str, name: &str) -> Program {
        Program {
            id: id.into(),
            name: name.into(),
            icon: Some("text-x-generic".into()),
        }
    }

    fn sample() -> OpenWith {
        let others = (0..12)
            .map(|i| {
                p(
                    &format!("app{}.desktop", 10 + i),
                    &format!("Program {i:02}"),
                )
            })
            .collect();
        OpenWith::new(
            "New Text Document.txt",
            vec![p("micro.desktop", "Micro"), p("vim.desktop", "Vim")],
            others,
        )
    }

    #[test]
    fn the_first_program_is_picked_under_two_headers() {
        let o = sample();
        assert_eq!(o.list.rows.len(), 16, "two headers and fourteen programs");
        assert_eq!(o.list.rows[0], Row::Group(RECOMMENDED.into()));
        assert_eq!(o.list.rows[3], Row::Group(OTHERS.into()));
        assert_eq!(o.list.selected, Some(1));
        assert_eq!(ROWS, 11);
        assert!(o.ok_enabled());
    }

    #[test]
    fn an_empty_group_gets_no_header() {
        let o = OpenWith::new("f", vec![], vec![p("a", "A")]);
        assert_eq!(
            o.list.rows,
            vec![
                Row::Group(OTHERS.into()),
                Row::Item {
                    name: "A".into(),
                    data: p("a", "A")
                }
            ]
        );
    }

    #[test]
    fn an_empty_chooser_has_ok_greyed_and_enter_does_nothing() {
        let mut o = OpenWith::new("f", vec![], vec![]);
        assert!(!o.ok_enabled());
        assert_eq!(o.key(Key::Enter), Event::None);
        assert_eq!(
            o.double_click(Some(Hit::List(grouplist::Hit::Row(0)))),
            Event::Redraw
        );
        for _ in 0..5 {
            o.key(Key::Tab { back: false });
            assert_ne!(o.focus, Focus::Ok, "OK never takes the focus while greyed");
        }
    }

    #[test]
    fn up_onto_a_header_greys_ok_and_enter_then_does_nothing() {
        let mut o = sample();
        assert_eq!(o.key(Key::Up), Event::Redraw);
        assert_eq!(o.list.selected, Some(0));
        assert!(!o.ok_enabled());
        assert_eq!(o.key(Key::Enter), Event::None);
        assert_eq!(o.key(Key::Up), Event::None, "the top row stops the walk");
    }

    #[test]
    fn home_end_and_the_page_keys_walk_the_list() {
        let mut o = sample();
        assert_eq!(o.key(Key::PageDown), Event::Redraw);
        assert_eq!((o.list.selected, o.list.top), (Some(12), 2), "a page on");
        assert_eq!(o.key(Key::End), Event::Redraw);
        assert_eq!(o.list.selected, Some(15));
        assert_eq!(o.key(Key::PageDown), Event::None, "the last row stops it");
        assert_eq!(o.key(Key::PageUp), Event::Redraw);
        assert_eq!(o.list.selected, Some(4));
        assert_eq!(o.key(Key::Home), Event::Redraw);
        assert_eq!((o.list.selected, o.list.top), (Some(0), 0));
        o.focus = Focus::Cancel;
        assert_eq!(o.key(Key::End), Event::None, "only the focused list walks");
        assert_eq!(o.list.selected, Some(0));
    }

    #[test]
    fn enter_chooses_the_pick_with_the_box() {
        let mut o = sample();
        assert_eq!(o.key(Key::Access('a')), Event::Redraw);
        assert!(o.always.on);
        assert_eq!(o.focus, Focus::Always);
        assert_eq!(
            o.key(Key::Enter),
            Event::Choose {
                id: "micro.desktop".into(),
                always: true
            }
        );
    }

    #[test]
    fn a_greyed_box_does_not_flip() {
        let mut o = sample();
        o.always = Check {
            on: true,
            enabled: false,
        };
        assert_eq!(o.key(Key::Access('a')), Event::None);
        assert_eq!(o.click(Some(Hit::Always)), Event::None);
        assert!(!o.holding(), "a greyed box is not held");
        assert!(o.always.on);
    }

    #[test]
    fn the_box_flips_on_the_release_over_it() {
        let mut o = sample();
        assert_eq!(o.click(Some(Hit::Always)), Event::Redraw, "the press arms");
        assert_eq!(o.pointer.of(Focus::Always), Interaction::Pressed);
        assert!(!o.always.on, "the press waits for the release");
        assert_eq!(o.focus, Focus::Always);
        assert_eq!(o.release(Some(Hit::Always)), Event::Redraw);
        assert!(o.always.on, "the release flips it");
        o.click(Some(Hit::Always));
        assert_eq!(o.motion(None), Event::Redraw, "dragged off");
        assert_eq!(o.pointer.of(Focus::Always), Interaction::Idle);
        assert_eq!(o.release(None), Event::Redraw);
        assert!(o.always.on, "released off it, unchanged");
        o.click(Some(Hit::Always));
        o.motion(None);
        assert_eq!(o.motion(Some(Hit::Always)), Event::Redraw, "back on");
        o.release(Some(Hit::Always));
        assert!(!o.always.on, "back on, the release flips it");
    }

    #[test]
    fn letters_walk_the_rows_and_wrap() {
        let mut o = sample();
        assert_eq!(o.key(Key::Char('o')), Event::Redraw);
        assert_eq!(
            o.list.selected,
            Some(3),
            "o lands on the Other Programs header"
        );
        assert_eq!(o.key(Key::Char('p')), Event::Redraw);
        assert_eq!(o.list.selected, Some(4));
        assert_eq!(o.key(Key::Char('z')), Event::None);
        o.list.select(15, ROWS);
        assert_eq!(o.key(Key::Char('p')), Event::Redraw);
        assert_eq!(
            o.list.selected,
            Some(4),
            "from the last P a p wraps to the first"
        );
    }

    #[test]
    fn double_click_on_a_program_chooses_it_and_on_a_header_only_picks() {
        let mut o = sample();
        let row = |i| Some(Hit::List(grouplist::Hit::Row(i)));
        assert_eq!(
            o.double_click(row(5)),
            Event::Choose {
                id: "app11.desktop".into(),
                always: false
            }
        );
        assert_eq!(o.double_click(row(3)), Event::Redraw);
        assert_eq!(o.list.selected, Some(3));
    }

    #[test]
    fn tab_skips_greyed_parts_and_escape_cancels() {
        let mut o = sample();
        o.browse_enabled = false;
        o.always.enabled = false;
        o.key(Key::Tab { back: false });
        assert_eq!(o.focus, Focus::Ok);
        o.key(Key::Tab { back: false });
        assert_eq!(o.focus, Focus::Cancel);
        assert_eq!(o.key(Key::Enter), Event::Cancel);
        o.key(Key::Tab { back: false });
        assert_eq!(o.focus, Focus::List);
        assert_eq!(o.key(Key::Escape), Event::Cancel);
    }

    #[test]
    fn browse_asks_the_host_unless_greyed() {
        let mut o = sample();
        assert_eq!(o.key(Key::Access('b')), Event::Browse);
        assert_eq!(o.click(Some(Hit::Browse)), Event::Redraw);
        assert_eq!(o.release(Some(Hit::Browse)), Event::Browse);
        o.browse_enabled = false;
        assert_eq!(o.click(Some(Hit::Browse)), Event::None);
        assert!(!o.holding(), "a greyed Browse... does not arm");
    }

    #[test]
    fn a_press_on_ok_arms_it_and_the_release_on_it_chooses() {
        let mut o = sample();
        assert_eq!(o.click(Some(Hit::Ok)), Event::Redraw, "the press only arms");
        assert_eq!(o.pointer.lit(), Some((Focus::Ok, Interaction::Pressed)));
        assert_eq!(
            o.release(Some(Hit::Ok)),
            Event::Choose {
                id: "micro.desktop".into(),
                always: false
            }
        );
        assert!(!o.holding());
        assert_eq!(o.release(Some(Hit::Ok)), Event::None, "nothing held");
    }

    #[test]
    fn a_release_off_the_pressed_button_only_pops_it_up() {
        let mut o = sample();
        o.click(Some(Hit::Ok));
        assert_eq!(o.release(Some(Hit::Cancel)), Event::Redraw);
        assert!(!o.holding(), "the release clears the held button");
        assert_eq!(o.click(Some(Hit::Cancel)), Event::Redraw);
        assert_eq!(o.focus, Focus::Cancel, "Cancel takes the focus");
        assert_eq!(o.release(None), Event::Redraw);
        assert_eq!(o.click(Some(Hit::Cancel)), Event::Redraw);
        assert_eq!(o.release(Some(Hit::Cancel)), Event::Cancel);
    }

    #[test]
    fn a_held_button_dragged_off_draws_at_rest() {
        let mut o = sample();
        o.click(Some(Hit::Ok));
        assert_eq!(o.motion(Some(Hit::Cancel)), Event::Redraw, "dragged off");
        assert_eq!(o.pointer.of(Focus::Ok), Interaction::Idle);
        assert_eq!(o.motion(None), Event::None, "still off it");
        assert_eq!(o.motion(Some(Hit::Ok)), Event::Redraw, "back on");
        assert_eq!(o.pointer.of(Focus::Ok), Interaction::Pressed);
        assert_eq!(o.motion(Some(Hit::Ok)), Event::None);
    }

    #[test]
    fn a_greyed_ok_does_not_arm() {
        let mut o = sample();
        o.key(Key::Up);
        assert!(!o.ok_enabled(), "a header greys OK");
        assert_eq!(o.click(Some(Hit::Ok)), Event::None);
        assert!(!o.holding());
    }

    #[test]
    fn a_browse_pick_joins_other_programs_once() {
        let mut o = OpenWith::new("f", vec![p("a", "A")], vec![]);
        o.add_program(Program {
            id: "/usr/bin/nano".into(),
            name: "nano".into(),
            icon: Some(PROGRAM_ICON.into()),
        });
        assert_eq!(o.list.rows.len(), 4, "a header was added for it");
        assert_eq!(o.list.rows[2], Row::Group(OTHERS.into()));
        assert_eq!(o.chosen().map(|p| p.id.as_str()), Some("/usr/bin/nano"));
        o.list.select(1, ROWS);
        o.add_program(Program {
            id: "/usr/bin/nano".into(),
            name: "nano".into(),
            icon: None,
        });
        assert_eq!(o.list.rows.len(), 4, "picked again, it is only selected");
        assert_eq!(o.list.selected, Some(3));
    }

    #[test]
    fn a_browse_pick_hands_the_focus_back_to_the_list() {
        let mut o = sample();
        o.click(Some(Hit::Browse));
        assert_eq!(o.release(Some(Hit::Browse)), Event::Browse);
        assert_eq!(o.focus, Focus::Browse);
        o.add_program(p("/usr/bin/nano", "nano"));
        assert_eq!(
            o.focus,
            Focus::List,
            "a new pick puts the focus on the list"
        );
        assert_eq!(
            o.key(Key::Enter),
            Event::Choose {
                id: "/usr/bin/nano".into(),
                always: false
            }
        );
        o.focus = Focus::Browse;
        o.add_program(p("vim.desktop", "Vim"));
        assert_eq!(
            o.focus,
            Focus::List,
            "a listed pick puts the focus on the list"
        );
    }

    #[test]
    fn a_refill_keeps_a_pick_that_is_still_listed() {
        let mut o = sample();
        assert!(o.select_id("vim.desktop"));
        o.set_programs(vec![p("vim.desktop", "Vim")], vec![p("x", "X")]);
        assert_eq!(o.chosen().map(|p| p.id.as_str()), Some("vim.desktop"));
    }

    #[test]
    fn a_refill_that_drops_the_pick_takes_the_first_program() {
        let mut o = sample();
        o.focus = Focus::Ok;
        o.set_programs(vec![], vec![p("x", "X")]);
        assert_eq!(o.chosen().map(|p| p.id.as_str()), Some("x"));
        o.set_programs(vec![], vec![]);
        assert_eq!(o.chosen(), None);
        assert_eq!(o.focus, Focus::List, "the focus leaves a greyed OK");
    }

    #[test]
    fn select_id_is_false_for_an_unlisted_id() {
        let mut o = sample();
        assert!(!o.select_id("nope"));
        assert_eq!(o.list.selected, Some(1));
    }

    #[test]
    fn rows_name_their_icons() {
        let o = sample();
        assert_eq!(icon_name(&o.list.rows[0]), Some("folder"));
        assert_eq!(icon_name(&o.list.rows[1]), Some("text-x-generic"));
    }

    use crate::theme::{FontConfig, Theme};

    fn text() -> TextRenderer {
        TextRenderer::load(&FontConfig::default())
    }

    #[test]
    fn a_blue_always_label_starts_16_px_in() {
        let o = OpenWith::new("a.txt", Vec::new(), Vec::new());
        let blue = crate::interfaces::theme_for(crate::interface::Interface::Blue);
        let l = o.layout(&text(), 11, (0, 0), &blue.look.controls());
        assert_eq!(l.check, Rect::new(27, 299, 13, 13));
        assert_eq!(l.check_label.x, 27 + 16);
        assert_eq!(o.hit(&l, 27 + 16, 305), Some(Hit::Always));
    }

    #[test]
    fn the_lists_thumb_drags_and_the_track_pages() {
        let t = text();
        let mut o = sample();
        let l = o.layout(&t, 11, (0, 0), &ControlMetrics::CLASSIC);
        let sb = l.list.scrollbar.unwrap();
        // 16 rows, 11 in view: a 120 px thumb on the 174 px track.
        let grab_y = sb.y + 16 + 4;
        let hit = o.hit(&l, sb.x + 8, grab_y);
        assert_eq!(hit, Some(Hit::List(grouplist::Hit::Thumb(4))));
        assert_eq!(o.click(hit), Event::None, "the press moves nothing");
        assert!(o.holding());
        assert_eq!(o.drag(&l, grab_y + 27), Some(Event::Redraw));
        assert_eq!(o.list.top, 3, "half the 54 px span: 3 rows of 5");
        assert_eq!(o.drag(&l, grab_y + 400), Some(Event::Redraw));
        assert_eq!(o.list.top, 5, "past the end: the last page");
        assert_eq!(o.release(None), Event::Redraw);
        assert!(!o.holding());
        assert_eq!(o.drag(&l, grab_y), None, "let go: nothing follows");
        assert_eq!(o.list.top, 5);
        let above = o.hit(&l, sb.x + 8, sb.y + 20);
        assert_eq!(above, Some(Hit::List(grouplist::Hit::PageUp)));
        assert_eq!(o.click(above), Event::Redraw);
        assert_eq!(o.list.top, 0, "a page back");
        assert!(!o.holding(), "the track grabs nothing");
    }

    #[test]
    fn the_wheel_over_the_list_scrolls_it() {
        let t = text();
        let mut o = sample();
        let l = o.layout(&t, 11, (0, 0), &ControlMetrics::CLASSIC);
        let picked = o.list.selected;
        let row = o.hit(&l, l.list.inner.x + 40, l.list.inner.y + 5);
        assert_eq!(o.wheel(row, 3), Event::Redraw);
        assert_eq!(o.list.top, 3);
        let sb = l.list.scrollbar.unwrap();
        let bar = o.hit(&l, sb.x + 8, sb.y + sb.height / 2);
        assert_eq!(o.wheel(bar, 3), Event::Redraw);
        assert_eq!(o.list.top, 5, "16 rows, 11 in view: 5 is the last top");
        assert_eq!(o.wheel(row, 3), Event::None, "at the bottom");
        assert_eq!(o.wheel(Some(Hit::Ok), -3), Event::None, "off the list");
        assert_eq!(o.list.top, 5);
        assert_eq!(o.wheel(row, -9), Event::Redraw);
        assert_eq!(o.list.top, 0);
        assert_eq!(o.list.selected, picked, "the pick stays");
    }

    #[test]
    fn the_grid_is_pinned_and_follows_the_origin() {
        let t = text();
        let o = sample();
        let l = o.layout(&t, 11, (0, 0), &ControlMetrics::CLASSIC);
        assert_eq!(l.client, Rect::new(0, 0, 390, 395));
        assert_eq!(l.icon, Rect::new(11, 11, 32, 32));
        assert_eq!((l.prompt.x, l.prompt.y), (54, 12));
        assert_eq!((l.file.x, l.file.y), (54, 38));
        assert_eq!(l.group, Rect::new(9, 66, 368, 288));
        assert_eq!(l.list.frame, Rect::new(24, 81, 338, 210));
        assert_eq!(l.check, Rect::new(27, 299, 13, 13));
        assert_eq!(l.check_label.x, 45);
        assert_eq!(l.browse, Rect::new(291, 320, 75, 23));
        assert_eq!(l.ok, Rect::new(210, 363, 75, 23));
        assert_eq!(l.cancel, Rect::new(291, 363, 75, 23));
        assert_eq!(
            l.list.scrollbar,
            Some(Rect::new(24 + 338 - 2 - 16, 83, 16, 206))
        );
        let moved = o.layout(&t, 11, (3, 21), &ControlMetrics::CLASSIC);
        assert_eq!(moved.list.frame, Rect::new(27, 102, 338, 210));
        assert_eq!(moved.ok, Rect::new(213, 384, 75, 23));
    }

    #[test]
    fn points_land_on_their_parts() {
        let t = text();
        let o = sample();
        let l = o.layout(&t, 11, (0, 0), &ControlMetrics::CLASSIC);
        let row3 = (l.list.inner.x + 40, l.list.inner.y + 3 * 18 + 5);
        assert_eq!(
            o.hit(&l, row3.0, row3.1),
            Some(Hit::List(grouplist::Hit::Row(3)))
        );
        assert_eq!(o.hit(&l, l.check.x + 6, l.check.y + 6), Some(Hit::Always));
        assert_eq!(
            o.hit(&l, l.check_label.x + 10, l.check_label.y + 6),
            Some(Hit::Always)
        );
        assert_eq!(o.hit(&l, 300, 330), Some(Hit::Browse));
        assert_eq!(o.hit(&l, 250, 369), Some(Hit::Ok));
        assert_eq!(o.hit(&l, 300, 369), Some(Hit::Cancel));
        assert_eq!(o.hit(&l, 5, 5), None);
    }

    #[test]
    fn it_draws_the_group_box_list_pick_and_greyed_box() {
        let theme = Theme::default();
        let cs = &theme.colors;
        let t = text();
        let mut o = sample();
        o.always = Check {
            on: true,
            enabled: false,
        };
        let l = o.layout(&t, theme.fonts.size_px, (0, 0), &theme.look.controls());
        let mut c = Canvas::new(CLIENT.0 as u32, CLIENT.1 as u32);
        o.draw(
            &mut c,
            &t,
            &classic_paint(cs, 1),
            &l,
            None,
            &[],
            Cues::NONE,
            theme.fonts.size_px,
        );
        let g = l.group;
        assert_eq!(
            c.get(g.x + 200, g.y),
            cs.button_shadow.argb(),
            "etched top line, shadow"
        );
        assert_eq!(
            c.get(g.x + 200, g.y + 1),
            cs.button_hilight.argb(),
            "etched top line, light"
        );
        assert_eq!(
            c.get(g.x + 12, g.y),
            cs.button_face.argb(),
            "the line breaks under the label"
        );
        let f = l.list.frame;
        assert_eq!(c.get(f.x, f.y), cs.button_shadow.argb(), "sunken list edge");
        assert_eq!(
            c.get(f.x + 200, f.y + 4 + 18 + 9),
            cs.window.argb(),
            "list inside is window colour"
        );
        let sel_y = l.list.inner.y + 18 + 9;
        // A px left of the label, inside the selection's lead-in padding,
        // so the check never lands on a glyph's own ink.
        assert_eq!(
            c.get(f.x + 44, sel_y),
            cs.hilight.argb(),
            "the picked row is highlighted"
        );
        let b = l.check;
        let has = |r: Rect, color: u32| {
            (r.x..r.right()).any(|x| (r.y..r.bottom()).any(|y| c.get(x, y) == color))
        };
        assert!(
            has(b.inset(2), cs.button_face.argb()),
            "a greyed box is face inside"
        );
        assert!(
            !has(b, cs.button_text.argb()),
            "a greyed box has no black mark"
        );
        assert!(
            !has(l.check_label, cs.button_text.argb()),
            "a greyed label is etched"
        );
    }

    #[test]
    fn a_held_ok_draws_pushed() {
        let theme = Theme::default();
        let cs = &theme.colors;
        let t = text();
        let mut o = sample();
        let l = o.layout(&t, theme.fonts.size_px, (0, 0), &theme.look.controls());
        let shot = |o: &OpenWith| {
            let mut c = Canvas::new(CLIENT.0 as u32, CLIENT.1 as u32);
            o.draw(
                &mut c,
                &t,
                &classic_paint(cs, 1),
                &l,
                None,
                &[],
                Cues::NONE,
                theme.fonts.size_px,
            );
            c.get(l.ok.x + 1, l.ok.y + 1)
        };
        assert_ne!(
            shot(&o),
            cs.button_shadow.argb(),
            "raised: light inside the ring"
        );
        o.click(Some(Hit::Ok));
        assert_eq!(
            shot(&o),
            cs.button_shadow.argb(),
            "pushed: shadow inside the ring"
        );
    }

    fn shot(o: &OpenWith, active: bool, cues: Cues) -> (Canvas, Layout) {
        let theme = Theme::default();
        let t = text();
        let l = o.layout(&t, theme.fonts.size_px, (0, 0), &theme.look.controls());
        let mut c = Canvas::new(CLIENT.0 as u32, CLIENT.1 as u32);
        let p = Paint {
            active,
            ..classic_paint(&theme.colors, 1)
        };
        o.draw(&mut c, &t, &p, &l, None, &[], cues, theme.fonts.size_px);
        (c, l)
    }

    #[test]
    fn an_inactive_chooser_draws_ok_without_the_default_ring() {
        let cs = Theme::default().colors;
        let o = sample();
        let (c, l) = shot(&o, true, Cues::NONE);
        assert_eq!(
            c.get(l.ok.x, l.ok.y + 10),
            cs.button_text.argb(),
            "active: the ring"
        );
        let (c, l) = shot(&o, false, Cues::NONE);
        assert_eq!(
            c.get(l.ok.x, l.ok.y + 10),
            cs.button_hilight.argb(),
            "inactive: a plain raised edge"
        );
    }

    /// A focused button or box wears its dotted rectangle only once the
    /// keyboard is used, and never in an inactive window; the default
    /// ring still follows the focus.
    #[test]
    fn focus_rectangles_wait_for_the_keyboard() {
        let cs = Theme::default().colors;
        let mut o = sample();
        let dotted =
            |c: &Canvas, r: Rect| any_in(c, Rect::new(r.x, r.y, r.width, 1), cs.button_text.argb());
        o.focus = Focus::Cancel;
        let (c, l) = shot(&o, true, Cues::ALL);
        let dots = l.cancel.inset(4);
        assert!(dotted(&c, dots), "with cues Cancel is dotted");
        let (c, _) = shot(&o, true, Cues::NONE);
        assert!(!dotted(&c, dots), "without cues no dots");
        assert_eq!(
            c.get(l.cancel.x, l.cancel.y + 10),
            cs.button_text.argb(),
            "the ring stays"
        );
        let (c, _) = shot(&o, false, Cues::ALL);
        assert!(!dotted(&c, dots), "inactive: no dots");
        o.focus = Focus::Always;
        let lr = l.check_label;
        let above = Rect::new(lr.x, lr.y - 1, lr.width, 1);
        let (c, _) = shot(&o, true, Cues::ALL);
        assert!(dotted(&c, above), "with cues the box's label is ringed");
        let (c, _) = shot(&o, true, Cues::NONE);
        assert!(!dotted(&c, above), "without cues the label has no ring");
    }

    #[test]
    fn underlines_wait_for_alt() {
        let mut o = sample();
        o.focus = Focus::Cancel;
        let focus = Cues {
            focus: true,
            keys: false,
        };
        let (bare, l) = shot(&o, true, focus);
        let (keyed, _) = shot(&o, true, Cues::ALL);
        let differs = |r: Rect| {
            (r.x..r.right())
                .any(|x| (r.y..r.bottom() + 2).any(|y| bare.get(x, y) != keyed.get(x, y)))
        };
        assert!(differs(l.group_label), "Programs' P");
        assert!(differs(l.check_label), "Always' A");
        assert!(differs(l.browse), "Browse...'s B");
        assert!(
            !differs(l.cancel) && !differs(l.list.frame),
            "the rest is alike"
        );
    }

    #[test]
    fn a_greyed_ok_and_browse_etch_their_labels() {
        let cs = Theme::default().colors;
        let mut o = sample();
        o.key(Key::Up);
        o.browse_enabled = false;
        assert!(!o.ok_enabled(), "a header greys OK");
        let (c, l) = shot(&o, true, Cues::NONE);
        for (r, name) in [(l.ok, "OK"), (l.browse, "Browse...")] {
            let label = r.inset(5);
            assert!(
                any_in(&c, label, cs.button_shadow.argb()),
                "{name}: shadow ink"
            );
            assert!(
                any_in(&c, label, cs.button_hilight.argb()),
                "{name}: white copy"
            );
            assert!(
                !any_in(&c, label, cs.button_text.argb()),
                "{name}: no black ink"
            );
        }
        assert!(
            any_in(&c, l.cancel.inset(5), cs.button_text.argb()),
            "Cancel stays live"
        );
    }

    #[test]
    fn a_live_box_is_white_inside() {
        let theme = Theme::default();
        let cs = &theme.colors;
        let t = text();
        let o = sample();
        let l = o.layout(&t, theme.fonts.size_px, (0, 0), &theme.look.controls());
        let mut c = Canvas::new(CLIENT.0 as u32, CLIENT.1 as u32);
        o.draw(
            &mut c,
            &t,
            &classic_paint(cs, 1),
            &l,
            None,
            &[],
            Cues::NONE,
            theme.fonts.size_px,
        );
        assert_eq!(c.get(l.check.x + 6, l.check.y + 6), cs.window.argb());
    }

    #[test]
    fn the_file_name_starts_at_x_90_after_the_file_label() {
        if !crate::text::tahoma_installed() {
            return;
        }
        let theme = Theme::default();
        let cs = &theme.colors;
        let t = text();
        let o = sample();
        let l = o.layout(&t, theme.fonts.size_px, (0, 0), &theme.look.controls());
        let mut c = Canvas::new(CLIENT.0 as u32, CLIENT.1 as u32);
        o.draw(
            &mut c,
            &t,
            &classic_paint(cs, 1),
            &l,
            None,
            &[],
            Cues::NONE,
            theme.fonts.size_px,
        );
        let ink =
            |x0: i32, x1: i32| any_in(&c, Rect::new(x0, 38, x1 - x0, 13), cs.button_text.argb());
        assert!(ink(54, 72), "\"File:\" draws from x 54");
        assert!(!ink(72, 90), "a gap from the label's end to x 90");
        assert!(ink(90, 91), "the name's N starts at x 90");
    }

    fn draw_at_2x(o: &OpenWith) -> (Canvas, Layout, ColorScheme) {
        let theme = Theme::default().scaled(2);
        let t = text();
        let px = theme.fonts.size_px;
        let l = o.layout(&t, px, (0, 0), &ControlMetrics::CLASSIC);
        let mut c = Canvas::new(2 * CLIENT.0 as u32, 2 * CLIENT.1 as u32);
        o.draw(
            &mut c,
            &t,
            &classic_paint(&theme.colors, 2),
            &l,
            None,
            &[],
            Cues::ALL,
            px,
        );
        (c, l, theme.colors)
    }

    fn any_in(c: &Canvas, r: Rect, color: u32) -> bool {
        (r.x..r.right()).any(|x| (r.y..r.bottom()).any(|y| c.get(x, y) == color))
    }

    #[test]
    fn at_scale_2_the_always_focus_ring_stays_inside_the_group() {
        let mut o = sample();
        o.focus = Focus::Always;
        let (c, l, cs) = draw_at_2x(&o);
        let g = l.group;
        let past = Rect::new(
            2 * g.right(),
            2 * (l.check.y - 2),
            2 * (CLIENT.0 - g.right()),
            2 * (l.check.height + 4),
        );
        assert!(
            !any_in(&c, past, cs.button_text.argb()),
            "no ring dots right of the group box (x {})",
            g.right()
        );
        let lr = l.check_label;
        let ring_top = Rect::new(2 * lr.x, 2 * (lr.y - 1), lr.width, 2);
        assert!(
            any_in(&c, ring_top, cs.button_text.argb()),
            "the ring's top edge runs over the label"
        );
    }

    #[test]
    fn the_pick_is_highlighted_only_with_the_focus() {
        let cs = Theme::default().colors;
        let mut o = sample();
        let pick = |(c, l): (Canvas, Layout)| c.get(l.list.frame.x + 44, l.list.inner.y + 27);
        assert_eq!(pick(shot(&o, true, Cues::NONE)), cs.hilight.argb());
        assert_eq!(
            pick(shot(&o, false, Cues::NONE)),
            cs.button_face.argb(),
            "inactive"
        );
        o.focus = Focus::Cancel;
        assert_eq!(
            pick(shot(&o, true, Cues::ALL)),
            cs.button_face.argb(),
            "focus elsewhere"
        );
    }

    #[test]
    fn the_tree_has_the_list_focused_and_ok_greyed_on_a_header() {
        let t = text();
        let mut o = sample();
        o.key(form::Key::Up);
        let l = o.layout(&t, 11, (0, 0), &ControlMetrics::CLASSIC);
        let mut tree = Tree::window(TITLE, CLIENT);
        let focus = o.access(&mut tree, &l);
        let n = tree.get(focus).unwrap();
        assert_eq!(
            n.label(),
            Some(RECOMMENDED),
            "the picked header row has the focus"
        );
        let ok = tree
            .nodes()
            .find(|(_, n)| n.label() == Some("OK"))
            .map(|(_, n)| n.clone())
            .unwrap();
        assert!(ok.is_disabled(), "OK is greyed on a header");
        assert!(
            tree.nodes().any(|(_, n)| n.role() == Role::CheckBox),
            "the box is a check box"
        );
    }
}
