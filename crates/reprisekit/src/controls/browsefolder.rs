//! The Move and Copy folder picker: a prompt over a folder tree, a hint, and
//! three buttons (Make New Folder, accept, Cancel). The host fills tree nodes as
//! they open and decides what can be picked, greying buttons via `set_enabled`.
//! A button takes focus on press and acts on release over it. A folder made by
//! Make New Folder opens in the tree's rename box; the default ring follows a
//! focused button, but Make New Folder takes it only from a pointer press.

use crate::access::{self, Tree};
use crate::classic::widgets::BarHit;
use crate::controls::form;
use crate::controls::metrics::ControlMetrics;
use crate::controls::treeview::{self, Path, TreeView};
use crate::cues::Cues;
use crate::look::Paint;
use crate::look::widgets::{draw_button, draw_size_grip};
use crate::pointer::Pointer;
use crate::raster::{Canvas, Rect};
use crate::text::{self, Align, TextRenderer};
use crate::textedit::{EditKey, TextEdit};
use crate::theme::ColorScheme;

pub const CLIENT: (i32, i32) = (318, 351);
pub const MIN: (i32, i32) = (310, 262);
pub const HINT: &str = "Click a plus sign to see the folders inside.";
pub const NEW_FOLDER_LABEL: &str = "&Make New Folder";

const PROMPT: Rect = Rect::new(15, 12, 290, 39);
const TREE: Rect = Rect::new(17, 60, 286, 208);
const HINT_RECT: Rect = Rect::new(15, 287, 290, 13);
const NEW_FOLDER: Rect = Rect::new(15, 317, 105, 23);
const ACCEPT: Rect = Rect::new(147, 317, 75, 23);
const CANCEL: Rect = Rect::new(228, 317, 75, 23);
const GRIP: i32 = 16;
const LINE_H: i32 = 13;
const HSCROLL_STEP: i32 = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    Tree,
    NewFolder,
    Accept,
    Cancel,
}

const TAB_ORDER: [Focus; 4] = [Focus::Tree, Focus::NewFolder, Focus::Accept, Focus::Cancel];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Tree(treeview::Hit),
    Edit,
    NewFolder,
    Accept,
    Cancel,
    Grip,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    None,
    Redraw,
    Expand(Path),
    Selected(Path),
    NewFolder(Path),
    /// The rename box closed on this node with this name; the host renames the
    /// folder, or reopens the box ([`BrowseFolder::reopen_edit`]) when it cannot.
    Rename(Path, String),
    Accept(Path),
    Cancel,
    Resize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Thumb {
    Vertical(i32),
    Horizontal(i32),
}

pub struct BrowseFolder {
    pub prompt: String,
    pub accept: String,
    pub tree: TreeView,
    pub focus: Focus,
    pub accept_enabled: bool,
    pub new_folder_enabled: bool,
    /// The button wearing the default ring while the tree or Make New Folder has the focus.
    default: Focus,
    size: (i32, i32),
    /// The keyboard cues it draws with: none until the host says the dialog was
    /// opened from the keyboard or [`BrowseFolder::cue`] brings them out.
    cues: Cues,
    /// `release`.
    thumb: Option<Thumb>,
    buttons: Pointer<Focus>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub prompt: Rect,
    pub tree: treeview::Layout,
    pub hint: Rect,
    pub new_folder: Rect,
    pub accept: Rect,
    pub cancel: Rect,
    pub grip: Rect,
    pub edit: Option<Rect>,
}

impl BrowseFolder {
    pub fn new(prompt: String, accept: String, tree: TreeView) -> BrowseFolder {
        let mut d = BrowseFolder {
            prompt,
            accept,
            tree,
            focus: Focus::Accept,
            accept_enabled: true,
            new_folder_enabled: true,
            default: Focus::Accept,
            size: CLIENT,
            cues: Cues::NONE,
            thumb: None,
            buttons: Pointer::new(),
        };
        d.set_focus(Focus::Accept);
        d
    }

    /// Move the focus, keeping the tree's pick drawn as focused only while the tree has it.
    pub fn set_focus(&mut self, f: Focus) {
        self.focus = f;
        self.default = Focus::Accept;
        self.sync_tree();
    }

    fn sync_tree(&mut self) {
        self.tree.inactive = self.focus != Focus::Tree;
        self.tree.focus_ring = self.focus == Focus::Tree && self.cues.focus;
    }

    /// Make New Folder made the folder at `path`: pick it, focus the tree and open the rename box on it.
    pub fn new_folder_made(&mut self, path: Path, lay: &Layout) {
        self.tree.selected = Some(path.clone());
        self.tree.follow(&lay.tree);
        let ring = self.default;
        self.set_focus(Focus::Tree);
        self.default = ring;
        self.tree.begin_edit(path);
    }

    /// The host refused a name for the node at `path`: reopen the box on the kept
    /// `name`, selected, with the default ring back on accept.
    pub fn reopen_edit(&mut self, path: Path, name: &str) {
        self.tree.edit = Some((path, TextEdit::new(name, true)));
        self.default = Focus::Accept;
    }

    /// Close the rename box, handing its node and name to the host.
    fn commit_edit(&mut self) -> Event {
        match self.tree.edit.take() {
            Some((path, ed)) => Event::Rename(path, ed.text.clone()),
            None => Event::None,
        }
    }

    pub fn cues(&self) -> Cues {
        self.cues
    }

    pub fn cue(&mut self, alt: bool) -> bool {
        if self.tree.edit.is_some() && !alt {
            return false;
        }
        let out = self.cues.key(alt);
        self.sync_tree();
        out
    }

    /// Grey or enable the accept and Make New Folder buttons for the current
    /// pick; greying the focused button hands the focus to the tree.
    pub fn set_enabled(&mut self, accept: bool, new_folder: bool) {
        self.accept_enabled = accept;
        self.new_folder_enabled = new_folder;
        if !self.focusable(self.focus) {
            self.set_focus(Focus::Tree);
        }
    }

    fn focusable(&self, f: Focus) -> bool {
        match f {
            Focus::Tree | Focus::Cancel => true,
            Focus::NewFolder => self.new_folder_enabled,
            Focus::Accept => self.accept_enabled,
        }
    }

    pub fn size(&self) -> (i32, i32) {
        self.size
    }

    /// The host's window took `size`: lay the parts out in it, no smaller than `MIN`.
    pub fn set_size(
        &mut self,
        size: (i32, i32),
        text: &TextRenderer,
        px: u32,
        cm: &ControlMetrics,
    ) {
        self.size = (size.0.max(MIN.0), size.1.max(MIN.1));
        let lay = self.layout(text, px, cm);
        self.tree.follow(&lay.tree);
    }

    pub fn layout(&self, text: &TextRenderer, px: u32, cm: &ControlMetrics) -> Layout {
        let (dw, dh) = (self.size.0 - CLIENT.0, self.size.1 - CLIENT.1);
        let grow = |r: Rect, w: i32, h: i32| Rect::new(r.x, r.y, r.width + w, r.height + h);
        let shift = |r: Rect, x: i32, y: i32| Rect::new(r.x + x, r.y + y, r.width, r.height);
        let mut lay = Layout {
            prompt: grow(PROMPT, dw, 0),
            tree: treeview::layout(&self.tree, text, px, grow(TREE, dw, dh), cm),
            hint: shift(grow(HINT_RECT, dw, 0), 0, dh),
            new_folder: shift(NEW_FOLDER, 0, dh),
            accept: shift(ACCEPT, dw, dh),
            cancel: shift(CANCEL, dw, dh),
            grip: Rect::new(self.size.0 - GRIP, self.size.1 - GRIP, GRIP, GRIP),
            edit: None,
        };
        lay.edit = self.tree.edit_bounds(&lay.tree, text, px);
        lay
    }

    pub fn hit(&self, lay: &Layout, x: i32, y: i32) -> Option<Hit> {
        if lay.edit.is_some_and(|r| r.contains(x, y)) {
            return Some(Hit::Edit);
        }
        if lay.new_folder.contains(x, y) {
            return Some(Hit::NewFolder);
        }
        if lay.accept.contains(x, y) {
            return Some(Hit::Accept);
        }
        if lay.cancel.contains(x, y) {
            return Some(Hit::Cancel);
        }
        if lay.grip.contains(x, y) {
            return Some(Hit::Grip);
        }
        match self.tree.hit(&lay.tree, x, y) {
            treeview::Hit::Nothing => None,
            h => Some(Hit::Tree(h)),
        }
    }

    pub fn click(&mut self, lay: &Layout, hit: Option<Hit>) -> Event {
        // A click anywhere but the rename box closes it and does nothing else yet: the rename may re-sort the tree. The host then replays the click.
        if self.tree.edit.is_some() {
            return match hit {
                Some(Hit::Edit) => Event::None,
                _ => self.commit_edit(),
            };
        }
        let Some(hit) = hit else {
            return Event::None;
        };
        let tl = &lay.tree;
        match hit {
            Hit::Tree(treeview::Hit::Row(i)) => {
                self.set_focus(Focus::Tree);
                match self.tree.click_row(i, tl) {
                    Some(p) => Event::Expand(p),
                    None => self.picked(),
                }
            }
            Hit::Tree(treeview::Hit::Expander(i)) => {
                self.set_focus(Focus::Tree);
                self.toggle(i, tl)
            }
            Hit::Tree(treeview::Hit::VBar(b)) => {
                let rows = tl.rows as i64;
                match b {
                    BarHit::Back => self.tree.scroll(-1, tl),
                    BarHit::Forward => self.tree.scroll(1, tl),
                    BarHit::PageBack => self.tree.scroll(-rows, tl),
                    BarHit::PageForward => self.tree.scroll(rows, tl),
                    BarHit::Thumb(grab) => {
                        self.thumb = Some(Thumb::Vertical(grab));
                        return Event::None;
                    }
                    BarHit::Track => false,
                };
                Event::Redraw
            }
            Hit::Tree(treeview::Hit::HBar(b)) => {
                let page = tl.body.width;
                match b {
                    BarHit::Back => self.tree.hscroll(-HSCROLL_STEP, tl),
                    BarHit::Forward => self.tree.hscroll(HSCROLL_STEP, tl),
                    BarHit::PageBack => self.tree.hscroll(-page, tl),
                    BarHit::PageForward => self.tree.hscroll(page, tl),
                    BarHit::Thumb(grab) => {
                        self.thumb = Some(Thumb::Horizontal(grab));
                        return Event::None;
                    }
                    BarHit::Track => false,
                };
                Event::Redraw
            }
            Hit::Tree(treeview::Hit::Nothing) | Hit::Edit => Event::None,
            Hit::NewFolder | Hit::Accept | Hit::Cancel => {
                let b = button_of(hit).expect("a button hit");
                if !self.focusable(b) {
                    return Event::None;
                }
                self.set_focus(b);
                self.default = b;
                self.buttons.press(Some(b), false);
                Event::Redraw
            }
            Hit::Grip => Event::Resize,
        }
    }

    pub fn double_click(&mut self, lay: &Layout, hit: Option<Hit>) -> Event {
        match hit {
            Some(Hit::Tree(treeview::Hit::Expander(_))) if self.tree.edit.is_none() => Event::None,
            _ => self.click(lay, hit),
        }
    }

    pub fn motion(&mut self, lay: &Layout, text: &TextRenderer, px: u32, x: i32, y: i32) -> Event {
        let tl = &lay.tree;
        let over = self
            .hit(lay, x, y)
            .and_then(button_of)
            .filter(|_| self.thumb.is_none());
        let lit = self.buttons.motion(over);
        let free = self.thumb.is_none() && !self.buttons.holding();
        let hot = if free {
            self.tree.hover(tl, text, px, x, y)
        } else {
            self.tree.unhover()
        };
        let moved = match self.thumb {
            Some(Thumb::Vertical(grab)) => {
                let top = self.tree.top_at(tl, y, grab);
                std::mem::replace(&mut self.tree.top, top) != top
            }
            Some(Thumb::Horizontal(grab)) => {
                let left = self.tree.left_at(tl, x, grab);
                std::mem::replace(&mut self.tree.left, left) != left
            }
            None => false,
        };
        if moved || lit || hot {
            Event::Redraw
        } else {
            Event::None
        }
    }

    pub fn release(&mut self) -> Event {
        self.thumb = None;
        let held = self.buttons.holding();
        match self.buttons.release() {
            Some(b) => self.press(b),
            None if held => Event::Redraw,
            None => Event::None,
        }
    }

    pub fn leave(&mut self) -> Event {
        if self.buttons.leave() | self.tree.unhover() {
            Event::Redraw
        } else {
            Event::None
        }
    }

    pub fn wheel(&mut self, lay: &Layout, x: i32, y: i32, rows: i32) -> Event {
        if lay.tree.frame.contains(x, y) && self.tree.scroll(rows as i64, &lay.tree) {
            Event::Redraw
        } else {
            Event::None
        }
    }

    pub fn key(&mut self, lay: &Layout, key: form::Key) -> Event {
        use form::Key;
        if self.tree.edit.is_some() {
            return match key {
                Key::Enter | Key::Tab { .. } => self.commit_edit(),
                Key::Escape => {
                    self.tree.edit = None;
                    Event::Redraw
                }
                Key::Edit(EditKey::Insert(s)) if s.contains('/') => Event::None,
                Key::Edit(e) => {
                    if let Some((_, ed)) = self.tree.edit.as_mut() {
                        ed.key(e);
                    }
                    Event::Redraw
                }
                _ => Event::None,
            };
        }
        let key = match key {
            Key::Edit(e) => match form::flat_key(&e) {
                Some(k) => k,
                None => return Event::None,
            },
            k => k,
        };
        match key {
            Key::Tab { back } => {
                let n = TAB_ORDER.len();
                let mut at = TAB_ORDER.iter().position(|&f| f == self.focus).unwrap_or(0);
                loop {
                    at = (if back { at + n - 1 } else { at + 1 }) % n;
                    if self.focusable(TAB_ORDER[at]) {
                        break;
                    }
                }
                self.set_focus(TAB_ORDER[at]);
                return Event::Redraw;
            }
            Key::Escape => return Event::Cancel,
            Key::Enter => {
                return match self.focus {
                    Focus::Cancel => Event::Cancel,
                    Focus::NewFolder if self.new_folder_enabled => self.press(Focus::NewFolder),
                    _ => self.press(Focus::Accept),
                };
            }
            Key::Space => return self.press(self.focus),
            Key::Access(c) if c.eq_ignore_ascii_case(&'m') => {
                return self.press(Focus::NewFolder);
            }
            _ => {}
        }
        if self.focus != Focus::Tree {
            return Event::None;
        }
        let tl = &lay.tree;
        let rows = tl.rows as i64;
        let before = self.tree.selected.clone();
        let moved = |d: &BrowseFolder| d.tree.selected != before;
        match key {
            Key::Up | Key::Down | Key::PageUp | Key::PageDown | Key::Home | Key::End => {
                let step = match key {
                    Key::Up => -1,
                    Key::Down => 1,
                    Key::PageUp => -rows,
                    Key::PageDown => rows,
                    Key::Home => i64::MIN / 2,
                    _ => i64::MAX / 2,
                };
                if self.tree.step(step, tl) && moved(self) {
                    self.picked()
                } else {
                    Event::None
                }
            }
            Key::Left | Key::Char('-') => {
                if !self.tree.collapse_selected(tl) {
                    Event::None
                } else if moved(self) {
                    self.picked()
                } else {
                    Event::Redraw
                }
            }
            Key::Right | Key::Char('+') => match self.tree.expand_selected(tl) {
                Some(p) => Event::Expand(p),
                None if moved(self) => self.picked(),
                None => Event::Redraw,
            },
            _ => Event::None,
        }
    }

    fn picked(&self) -> Event {
        match &self.tree.selected {
            Some(p) => Event::Selected(p.clone()),
            None => Event::Redraw,
        }
    }

    /// Open or close visible row `i`: `Expand` when it opened with no children
    /// loaded. Closing a row above the pick moves the pick up to it.
    fn toggle(&mut self, i: usize, tl: &treeview::Layout) -> Event {
        let Some(row) = self.tree.visible().get(i).map(|r| r.path.clone()) else {
            return Event::None;
        };
        if let Some(p) = self.tree.toggle(i, tl) {
            return Event::Expand(p);
        }
        let closed = self.tree.node(&row).is_some_and(|n| !n.expanded);
        let hidden = self
            .tree
            .selected
            .as_ref()
            .is_some_and(|s| s.len() > row.len() && s.starts_with(&row));
        if closed && hidden {
            self.tree.selected = Some(row.clone());
            self.tree.follow(tl);
            return Event::Selected(row);
        }
        Event::Redraw
    }

    fn press(&self, b: Focus) -> Event {
        let sel = self.tree.selected.clone();
        match b {
            Focus::Tree => Event::None,
            Focus::Cancel => Event::Cancel,
            Focus::NewFolder if self.new_folder_enabled => {
                sel.map_or(Event::None, Event::NewFolder)
            }
            Focus::Accept if self.accept_enabled => sel.map_or(Event::None, Event::Accept),
            Focus::NewFolder | Focus::Accept => Event::None,
        }
    }

    pub fn draw(&self, c: &mut Canvas, text: &TextRenderer, p: &Paint, lay: &Layout, px: u32) {
        let cs = p.cs;
        let s = c.scale();
        c.fill_rect(c.bounds(), cs.button_face);
        draw_lines(c, text, cs, lay.prompt, &self.prompt, px, s);
        treeview::draw(c, text, &Paint { s, ..*p }, &self.tree, &lay.tree, None, px);
        draw_lines(c, text, cs, lay.hint, HINT, px, s);
        draw_size_grip(c, &Paint { s, ..*p }, lay.grip.scaled(s));
        let ring = |f: Focus| match self.focus {
            Focus::Tree | Focus::NewFolder => f == self.default,
            focus => f == focus,
        };
        let buttons = [
            (
                lay.new_folder,
                NEW_FOLDER_LABEL,
                self.new_folder_enabled,
                Focus::NewFolder,
            ),
            (
                lay.accept,
                self.accept.as_str(),
                self.accept_enabled,
                Focus::Accept,
            ),
            (lay.cancel, "Cancel", true, Focus::Cancel),
        ];
        for (r, label, enabled, f) in buttons {
            draw_button(
                c,
                text,
                &Paint { s, ..*p },
                r.scaled(s),
                &self.cues.label(label),
                ring(f),
                self.buttons.of(f),
                enabled,
                self.cues.focus && self.focus == f,
                px,
            );
        }
    }

    pub fn access(&self, tree: &mut Tree, text: &TextRenderer, px: u32, cm: &ControlMetrics) {
        let root = access::ROOT;
        let lay = self.layout(text, px, cm);
        tree.push(
            root,
            "prompt",
            access::label(lay.prompt, &self.prompt, true),
        );
        let tv = treeview::access(&self.tree, &lay.tree, text, px, tree, root, "tree");
        let new_folder = tree.push(
            root,
            "new folder",
            access::button(
                lay.new_folder,
                NEW_FOLDER_LABEL,
                self.new_folder_enabled,
                false,
            ),
        );
        let accept = tree.push(
            root,
            "accept",
            access::button(lay.accept, &self.accept, self.accept_enabled, true),
        );
        let cancel = tree.push(
            root,
            "cancel",
            access::button(lay.cancel, "Cancel", true, false),
        );
        tree.push(root, "hint", access::label(lay.hint, HINT, true));
        tree.set_focus(match self.focus {
            Focus::Tree => tv,
            Focus::NewFolder => new_folder,
            Focus::Accept => accept,
            Focus::Cancel => cancel,
        });
    }
}

fn button_of(hit: Hit) -> Option<Focus> {
    match hit {
        Hit::NewFolder => Some(Focus::NewFolder),
        Hit::Accept => Some(Focus::Accept),
        Hit::Cancel => Some(Focus::Cancel),
        Hit::Tree(_) | Hit::Edit | Hit::Grip => None,
    }
}

fn draw_lines(
    c: &mut Canvas,
    text: &TextRenderer,
    cs: &ColorScheme,
    r: Rect,
    body: &str,
    px: u32,
    s: i32,
) {
    let lines = text::wrap(text, body, r.width, px);
    for (i, line) in lines.iter().take((r.height / LINE_H) as usize).enumerate() {
        let lr = Rect::new(r.x, r.y + LINE_H * i as i32, r.width, LINE_H).scaled(s);
        text.draw(c, lr, line, false, cs.button_text, Align::Left, px);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controls::treeview::{Node, Style};
    use crate::look::classic_paint;
    use crate::theme::{FontConfig, Theme};

    fn text() -> TextRenderer {
        TextRenderer::load(&FontConfig::default())
    }

    fn sample() -> BrowseFolder {
        let mut desk = Node::new("Desktop", true);
        desk.expanded = true;
        desk.children = vec![Node::new("My Documents", true), Node::new("Stuff", false)];
        let mut tv = TreeView::new(vec![desk], Style::Picker);
        tv.selected = Some(vec![0, 1]);
        BrowseFolder::new(
            "Pick the folder to move 'a.txt' to, then click Move.".into(),
            "Move".into(),
            tv,
        )
    }

    #[test]
    fn the_parts_sit_at_their_measured_rects() {
        let d = sample();
        let lay = d.layout(&text(), 11, &ControlMetrics::CLASSIC);
        assert_eq!(lay.tree.frame, Rect::new(17, 60, 286, 208), "tree frame");
        assert_eq!(
            lay.new_folder,
            Rect::new(15, 317, 105, 23),
            "Make New Folder"
        );
        assert_eq!(lay.accept, Rect::new(147, 317, 75, 23), "accept");
        assert_eq!(lay.cancel, Rect::new(228, 317, 75, 23), "Cancel");
        assert_eq!(
            (lay.prompt.x, lay.prompt.y, lay.prompt.width),
            (15, 12, 290),
            "prompt column"
        );
        assert_eq!((lay.hint.x, lay.hint.y), (15, 287), "hint line");
    }

    #[test]
    fn the_parts_follow_the_client_area() {
        let t = text();
        let cm = ControlMetrics::CLASSIC;
        let mut d = sample();
        d.set_size((478, 471), &t, 11, &cm);
        let lay = d.layout(&t, 11, &cm);
        assert_eq!(lay.tree.frame, Rect::new(17, 60, 446, 328), "tree");
        assert_eq!(lay.prompt.width, 450, "prompt column");
        assert_eq!((lay.hint.y, lay.hint.width), (407, 450), "hint");
        assert_eq!(
            lay.new_folder,
            Rect::new(15, 437, 105, 23),
            "Make New Folder"
        );
        assert_eq!(lay.accept, Rect::new(307, 437, 75, 23), "accept");
        assert_eq!(lay.cancel, Rect::new(388, 437, 75, 23), "Cancel");
        assert_eq!(lay.grip, Rect::new(462, 455, 16, 16), "grip");
        d.set_size((100, 100), &t, 11, &cm);
        assert_eq!(d.size(), MIN, "no smaller than 310x262");
        let lay = d.layout(&t, 11, &cm);
        assert_eq!(
            lay.tree.frame,
            Rect::new(17, 60, 278, 119),
            "tree at the least"
        );
        assert_eq!(lay.hint.y, 198, "hint at the least");
        assert_eq!(
            (lay.new_folder.y, lay.accept.x, lay.cancel.x),
            (228, 139, 220),
            "buttons at the least"
        );
    }

    #[test]
    fn the_size_grip_fills_the_corner_and_resizes() {
        let theme = Theme::default();
        let cs = &theme.colors;
        let t = text();
        let mut d = sample();
        let lay = d.layout(&t, 11, &ControlMetrics::CLASSIC);
        let mut c = Canvas::new(CLIENT.0 as u32, CLIENT.1 as u32);
        d.draw(&mut c, &t, &classic_paint(cs, 1), &lay, 11);
        let (w, h) = CLIENT;
        assert_eq!(
            c.get(w - 12, h - 1),
            cs.button_hilight.argb(),
            "bottom row, x 306"
        );
        assert_eq!(
            c.get(w - 11, h - 1),
            cs.button_shadow.argb(),
            "bottom row, x 307"
        );
        assert_eq!(
            c.get(w - 1, h - 12),
            cs.button_hilight.argb(),
            "right column, y 339"
        );
        assert_eq!(
            c.get(w - 1, h - 1),
            cs.button_face.argb(),
            "the corner itself"
        );
        let hit = d.hit(&lay, w - 4, h - 4);
        assert_eq!(hit, Some(Hit::Grip));
        assert_eq!(d.click(&lay, hit), Event::Resize);
        assert_eq!(
            d.hit(&lay, 302, 336),
            Some(Hit::Cancel),
            "Cancel's edge wins over the grip"
        );
    }

    #[test]
    fn it_opens_with_the_focus_on_accept_and_the_tree_inactive() {
        let d = sample();
        assert_eq!(d.focus, Focus::Accept);
        assert!(d.tree.inactive);
    }

    #[test]
    fn tab_walks_tree_new_folder_accept_cancel() {
        let mut d = sample();
        let lay = d.layout(&text(), 11, &ControlMetrics::CLASSIC);
        d.focus = Focus::Tree;
        let mut seen = vec![];
        for _ in 0..4 {
            d.key(&lay, form::Key::Tab { back: false });
            seen.push(d.focus);
        }
        assert_eq!(
            seen,
            [Focus::NewFolder, Focus::Accept, Focus::Cancel, Focus::Tree]
        );
        assert!(!d.tree.inactive, "the tree has the focus again");
        d.key(&lay, form::Key::Tab { back: true });
        assert_eq!(d.focus, Focus::Cancel);
        assert!(d.tree.inactive);
    }

    #[test]
    fn enter_accepts_the_pick_and_escape_cancels() {
        let mut d = sample();
        let lay = d.layout(&text(), 11, &ControlMetrics::CLASSIC);
        assert_eq!(d.key(&lay, form::Key::Enter), Event::Accept(vec![0, 1]));
        assert_eq!(d.key(&lay, form::Key::Escape), Event::Cancel);
        d.focus = Focus::Cancel;
        assert_eq!(
            d.key(&lay, form::Key::Enter),
            Event::Cancel,
            "Enter on Cancel cancels"
        );
    }

    #[test]
    fn space_presses_the_focused_button() {
        let mut d = sample();
        let lay = d.layout(&text(), 11, &ControlMetrics::CLASSIC);
        d.focus = Focus::NewFolder;
        assert_eq!(d.key(&lay, form::Key::Space), Event::NewFolder(vec![0, 1]));
    }

    #[test]
    fn a_greyed_accept_ignores_enter_and_clicks() {
        let mut d = sample();
        let lay = d.layout(&text(), 11, &ControlMetrics::CLASSIC);
        d.set_enabled(false, true);
        assert_eq!(d.key(&lay, form::Key::Enter), Event::None);
        assert_eq!(d.click(&lay, Some(Hit::Accept)), Event::None);
        d.set_enabled(true, false);
        assert_eq!(d.click(&lay, Some(Hit::NewFolder)), Event::None);
    }

    #[test]
    fn a_click_on_a_row_picks_it_and_opens_it() {
        let mut d = sample();
        let lay = d.layout(&text(), 11, &ControlMetrics::CLASSIC);
        let r = d.hit(&lay, lay.tree.body.x + 40, lay.tree.body.y + 17 + 8);
        assert_eq!(r, Some(Hit::Tree(treeview::Hit::Row(1))));
        assert_eq!(d.click(&lay, r), Event::Expand(vec![0, 0]));
        assert_eq!(d.tree.selected, Some(vec![0, 0]));
        assert!(d.tree.node(&[0, 0]).is_some_and(|n| n.expanded));
        assert_eq!(d.focus, Focus::Tree);
        assert!(!d.tree.inactive);
        let r = Some(Hit::Tree(treeview::Hit::Row(2)));
        assert_eq!(d.click(&lay, r), Event::Selected(vec![0, 1]), "a leaf");
    }

    #[test]
    fn a_double_click_turns_a_plus_box_once() {
        let mut d = sample();
        let lay = d.layout(&text(), 11, &ControlMetrics::CLASSIC);
        let docs = |d: &BrowseFolder| d.tree.node(&[0, 0]).is_some_and(|n| n.expanded);
        let row = Some(Hit::Tree(treeview::Hit::Row(1)));
        assert_eq!(d.click(&lay, row), Event::Expand(vec![0, 0]));
        d.tree.node_mut(&[0, 0]).unwrap().children = vec![Node::new("Music", false)];
        let lay = d.layout(&text(), 11, &ControlMetrics::CLASSIC);
        assert_eq!(d.double_click(&lay, row), Event::Selected(vec![0, 0]));
        assert!(docs(&d), "the row stays open");
        let plus = Some(Hit::Tree(treeview::Hit::Expander(1)));
        assert_eq!(d.click(&lay, plus), Event::Redraw);
        assert!(!docs(&d), "the first press closes it");
        assert_eq!(d.double_click(&lay, plus), Event::None);
        assert!(!docs(&d), "the second leaves it closed");
        assert_eq!(d.click(&lay, plus), Event::Redraw);
        assert!(docs(&d), "a later single click opens it");
    }

    #[test]
    fn right_on_an_unfilled_node_asks_to_expand() {
        let mut d = sample();
        let lay = d.layout(&text(), 11, &ControlMetrics::CLASSIC);
        d.focus = Focus::Tree;
        d.key(&lay, form::Key::Up);
        assert_eq!(d.tree.selected, Some(vec![0, 0]));
        assert_eq!(d.key(&lay, form::Key::Right), Event::Expand(vec![0, 0]));
        assert_eq!(
            d.key(&lay, form::Key::Char('+')),
            Event::Redraw,
            "already open"
        );
    }

    #[test]
    fn the_row_under_the_pointer_is_hot() {
        let t = text();
        let mut d = sample();
        let lay = d.layout(&t, 11, &ControlMetrics::CLASSIC);
        let (x, y) = (lay.tree.body.x + 40, lay.tree.body.y + 17 + 8);
        assert_eq!(d.motion(&lay, &t, 11, x, y), Event::Redraw);
        assert_eq!(d.tree.hot, Some(1));
        assert_eq!(d.motion(&lay, &t, 11, x, y), Event::None, "no change");
        assert_eq!(d.leave(), Event::Redraw);
        assert_eq!(d.tree.hot, None);
        d.motion(&lay, &t, 11, x, y);
        let cancel = d.hit(&lay, lay.cancel.x + 5, lay.cancel.y + 5);
        d.click(&lay, cancel);
        d.motion(&lay, &t, 11, x, y);
        assert_eq!(d.tree.hot, None, "not while Cancel is held");
    }

    #[test]
    fn collapsing_an_ancestor_of_the_pick_picks_the_closed_row() {
        let mut d = sample();
        let lay = d.layout(&text(), 11, &ControlMetrics::CLASSIC);
        let r = Some(Hit::Tree(treeview::Hit::Expander(0)));
        assert_eq!(d.click(&lay, r), Event::Selected(vec![0]));
        assert_eq!(d.tree.selected, Some(vec![0]), "the pick is the Desktop");
    }

    #[test]
    fn dialog_pixels() {
        let theme = Theme::default();
        let cs = &theme.colors;
        let t = text();
        let d = sample();
        let lay = d.layout(&t, 11, &ControlMetrics::CLASSIC);
        let mut c = Canvas::new(CLIENT.0 as u32, CLIENT.1 as u32);
        d.draw(&mut c, &t, &classic_paint(cs, 1), &lay, 11);
        assert_eq!(
            c.get(17, 100),
            cs.button_shadow.argb(),
            "tree frame outer left at x 17"
        );
        assert_eq!(
            c.get(18, 100),
            cs.button_dk_shadow.argb(),
            "tree frame inner left at x 18"
        );
        assert_eq!(
            c.get(301, 100),
            cs.button_light.argb(),
            "tree frame inner right at x 301"
        );
        assert_eq!(
            c.get(147, 325),
            cs.button_text.argb(),
            "accept's default ring at x 147"
        );
        assert_eq!(
            c.get(15, 325),
            cs.button_hilight.argb(),
            "Make New Folder's raised edge at x 15"
        );
        let inked = |y0: i32, y1: i32| {
            (y0..y1).any(|y| (14..300).any(|x| c.get(x, y) == cs.button_text.argb()))
        };
        assert!(inked(14, 36), "prompt ink between y 14 and 35");
        assert!(inked(289, 300), "hint ink between y 289 and 299");
        let pick_y = lay.tree.body.y + 17 + 17 + 8; // row 2: Stuff
        let pick_x = lay.tree.body.x + 42; // the band, a px left of the label at 43
        assert_eq!(
            c.get(pick_x, pick_y),
            cs.button_face.argb(),
            "the pick is inactive"
        );
    }

    #[test]
    fn the_cues_wait_for_the_keyboard() {
        let theme = Theme::default();
        let cs = &theme.colors;
        let t = text();
        let mut d = sample();
        let lay = d.layout(&t, 11, &ControlMetrics::CLASSIC);
        let shot = |d: &BrowseFolder| {
            let mut c = Canvas::new(CLIENT.0 as u32, CLIENT.1 as u32);
            d.draw(&mut c, &t, &classic_paint(cs, 1), &lay, 11);
            c
        };
        let dots = |c: &Canvas| {
            let r = lay.accept.inset(4);
            (r.x..r.right()).any(|x| c.get(x, r.y) == cs.button_text.argb())
        };
        let bare = shot(&d);
        assert!(!dots(&bare), "no rectangle before a key");
        assert!(d.cue(false), "a key brings the rectangle out");
        let focused = shot(&d);
        assert!(dots(&focused), "the rectangle after a key");
        assert!(d.cue(true), "Alt brings the underline out");
        let keyed = shot(&d);
        let r = lay.new_folder;
        let underlined = (r.x..r.right())
            .any(|x| (r.y..r.bottom()).any(|y| focused.get(x, y) != keyed.get(x, y)));
        assert!(underlined, "Alt underlines Make New Folder's M");
        assert!(!d.cue(true), "the cues stay out");
    }

    #[test]
    fn the_trees_ring_waits_for_the_keyboard() {
        let mut d = sample();
        d.set_focus(Focus::Tree);
        assert!(!d.tree.focus_ring, "a click leaves the pick unringed");
        d.cue(false);
        assert!(d.tree.focus_ring, "a key rings it");
        d.set_focus(Focus::Cancel);
        assert!(!d.tree.focus_ring, "not while the tree lacks the focus");
    }

    #[test]
    fn a_click_on_make_new_folder_focuses_it() {
        let mut d = sample();
        let lay = d.layout(&text(), 11, &ControlMetrics::CLASSIC);
        d.set_focus(Focus::Tree);
        assert_eq!(d.click(&lay, Some(Hit::NewFolder)), Event::Redraw);
        assert_eq!(d.focus, Focus::NewFolder);
        assert_eq!(d.release(), Event::NewFolder(vec![0, 1]));
        d.set_enabled(true, false);
        d.set_focus(Focus::Tree);
        assert_eq!(d.click(&lay, Some(Hit::NewFolder)), Event::None);
        assert_eq!(d.focus, Focus::Tree, "a greyed button leaves the focus");
        assert_eq!(d.release(), Event::None, "nor is it held");
    }

    #[test]
    fn a_button_acts_on_the_release_over_it() {
        let theme = Theme::default();
        let cs = &theme.colors;
        let t = text();
        let mut d = sample();
        let lay = d.layout(&t, 11, &ControlMetrics::CLASSIC);
        let edge = |d: &BrowseFolder| {
            let mut c = Canvas::new(CLIENT.0 as u32, CLIENT.1 as u32);
            d.draw(&mut c, &t, &classic_paint(cs, 1), &lay, 11);
            c.get(229, 325)
        };
        let (x, y) = (lay.cancel.x + 10, lay.cancel.y + 10);
        let hit = d.hit(&lay, x, y);
        assert_eq!(d.click(&lay, hit), Event::Redraw, "the press only holds it");
        assert_eq!(d.focus, Focus::Cancel, "and focuses it");
        assert_eq!(edge(&d), cs.button_shadow.argb(), "pushed while held");
        assert_eq!(d.motion(&lay, &t, 11, 5, 5), Event::Redraw, "dragged off");
        assert_eq!(edge(&d), cs.button_hilight.argb(), "up again off it");
        assert_eq!(d.release(), Event::Redraw, "released off it");
        assert_eq!(d.click(&lay, hit), Event::Redraw);
        assert_eq!(d.release(), Event::Cancel, "released on it");
        let hit = d.hit(&lay, lay.accept.x + 10, lay.accept.y + 10);
        d.click(&lay, hit);
        assert_eq!(d.release(), Event::Accept(vec![0, 1]));
    }

    #[test]
    fn a_made_folder_is_picked_and_the_ring_stays_where_the_press_put_it() {
        let theme = Theme::default();
        let cs = &theme.colors;
        let t = text();
        let mut d = sample();
        let lay = d.layout(&t, 11, &ControlMetrics::CLASSIC);
        let ring_x = |d: &BrowseFolder| {
            let mut c = Canvas::new(CLIENT.0 as u32, CLIENT.1 as u32);
            d.draw(&mut c, &t, &classic_paint(cs, 1), &lay, 11);
            [15, 147].map(|x| c.get(x, 325) == cs.button_text.argb())
        };
        let on_it = d.hit(&lay, lay.new_folder.x + 5, lay.new_folder.y + 5);
        d.click(&lay, on_it);
        assert_eq!(ring_x(&d), [true, false], "the press moves the ring");
        assert_eq!(d.release(), Event::NewFolder(vec![0, 1]));
        d.new_folder_made(vec![0, 0], &lay);
        assert_eq!(d.tree.selected, Some(vec![0, 0]));
        assert_eq!(d.focus, Focus::Tree);
        assert_eq!(ring_x(&d), [true, false], "the ring on Make New Folder");
        d.set_focus(Focus::Cancel);
        d.set_focus(Focus::Tree);
        assert_eq!(ring_x(&d), [false, true], "back on the accept button");

        let mut k = sample();
        k.set_focus(Focus::Tree);
        k.key(&lay, form::Key::Tab { back: false });
        assert_eq!(k.focus, Focus::NewFolder);
        assert_eq!(ring_x(&k), [false, true], "tabbed to: the ring stays");
        assert_eq!(k.key(&lay, form::Key::Space), Event::NewFolder(vec![0, 1]));
        k.new_folder_made(vec![0, 0], &lay);
        assert_eq!(ring_x(&k), [false, true], "and stays while it is named");
    }

    #[test]
    fn a_made_folder_is_named_in_the_rename_box() {
        use crate::textedit::EditKey;
        let t = text();
        let mut d = sample();
        let lay = d.layout(&t, 11, &ControlMetrics::CLASSIC);
        d.new_folder_made(vec![0, 1], &lay);
        let ed = |d: &BrowseFolder| {
            d.tree
                .edit
                .as_ref()
                .map(|(p, e)| (p.clone(), e.text.clone()))
        };
        assert_eq!(ed(&d), Some((vec![0, 1], "Stuff".into())), "open on it");
        let lay = d.layout(&t, 11, &ControlMetrics::CLASSIC);
        assert!(lay.edit.is_some(), "the box is laid out");
        for c in ["P", "i", "x"] {
            d.key(&lay, form::Key::Edit(EditKey::Insert(c.into())));
        }
        assert_eq!(d.key(&lay, form::Key::Space), Event::None, "no button");
        assert_eq!(d.key(&lay, form::Key::Access('m')), Event::None);
        assert_eq!(
            d.key(&lay, form::Key::Enter),
            Event::Rename(vec![0, 1], "Pix".into())
        );
        assert_eq!(ed(&d), None, "closed");
        assert_eq!(d.focus, Focus::Tree, "the tree keeps the focus");
    }

    #[test]
    fn the_rename_box_closes_on_escape_or_a_click_elsewhere() {
        use crate::textedit::EditKey;
        let t = text();
        let mut d = sample();
        let lay = d.layout(&t, 11, &ControlMetrics::CLASSIC);
        d.new_folder_made(vec![0, 1], &lay);
        let lay = d.layout(&t, 11, &ControlMetrics::CLASSIC);
        d.key(&lay, form::Key::Edit(EditKey::Insert("x".into())));
        assert_eq!(d.key(&lay, form::Key::Escape), Event::Redraw);
        assert!(d.tree.edit.is_none(), "Escape keeps the old name");
        d.reopen_edit(vec![0, 1], "Pix");
        let lay = d.layout(&t, 11, &ControlMetrics::CLASSIC);
        let r = lay.edit.expect("the box");
        let inside = d.hit(&lay, r.x + 2, r.y + 8);
        assert_eq!(inside, Some(Hit::Edit));
        assert_eq!(d.click(&lay, inside), Event::None);
        assert!(d.tree.edit.is_some(), "still open");
        let cancel = d.hit(&lay, lay.cancel.x + 5, lay.cancel.y + 5);
        assert_eq!(
            d.click(&lay, cancel),
            Event::Rename(vec![0, 1], "Pix".into())
        );
        assert_eq!(d.release(), Event::None, "the click held no button");
    }

    #[test]
    fn the_rename_box_drops_a_typed_slash() {
        use crate::textedit::EditKey;
        let t = text();
        let mut d = sample();
        let lay = d.layout(&t, 11, &ControlMetrics::CLASSIC);
        d.new_folder_made(vec![0, 1], &lay);
        let lay = d.layout(&t, 11, &ControlMetrics::CLASSIC);
        for k in ["q", "/", "z", "a/b"] {
            d.key(&lay, form::Key::Edit(EditKey::Insert(k.into())));
        }
        assert_eq!(
            d.key(&lay, form::Key::Enter),
            Event::Rename(vec![0, 1], "qz".into())
        );
    }

    #[test]
    fn typing_in_the_rename_box_brings_out_no_cue() {
        use crate::textedit::EditKey;
        let theme = Theme::default();
        let cs = &theme.colors;
        let t = text();
        let naming = || {
            let mut d = sample();
            let lay = d.layout(&t, 11, &ControlMetrics::CLASSIC);
            d.new_folder_made(vec![0, 1], &lay);
            d
        };
        let (mut d, mut quiet) = (naming(), naming());
        let lay = d.layout(&t, 11, &ControlMetrics::CLASSIC);
        for k in ["q", "z"] {
            let typed = || form::Key::Edit(EditKey::Insert(k.into()));
            assert!(!d.cue(false), "{k} brings out no cue");
            d.key(&lay, typed());
            quiet.key(&lay, typed());
        }
        assert_eq!(d.cues(), Cues::NONE);
        assert!(!d.tree.focus_ring, "the pick wears no rectangle");
        let shot = |d: &BrowseFolder| {
            let mut c = Canvas::new(CLIENT.0 as u32, CLIENT.1 as u32);
            d.draw(&mut c, &t, &classic_paint(cs, 1), &lay, 11);
            c
        };
        let (typed, untouched) = (shot(&d), shot(&quiet));
        let same =
            (0..CLIENT.1).all(|y| (0..CLIENT.0).all(|x| typed.get(x, y) == untouched.get(x, y)));
        assert!(same, "the keys drew no dots");
        assert_eq!(
            d.key(&lay, form::Key::Enter),
            Event::Rename(vec![0, 1], "qz".into())
        );
        assert!(d.cue(false), "with the box closed a key brings the cue out");
    }

    #[test]
    fn a_refused_name_reopens_the_box_on_the_kept_name() {
        let t = text();
        let mut d = sample();
        let lay = d.layout(&t, 11, &ControlMetrics::CLASSIC);
        let mf = d.hit(&lay, lay.new_folder.x + 5, lay.new_folder.y + 5);
        d.click(&lay, mf);
        d.new_folder_made(vec![0, 1], &lay);
        assert_eq!(d.default, Focus::NewFolder, "the press put the ring there");
        d.reopen_edit(vec![0, 1], "Stuff");
        let (path, ed) = d.tree.edit.as_ref().expect("the box is open");
        assert_eq!(path, &vec![0, 1]);
        assert_eq!(ed.selected_text(), "Stuff", "the kept name, selected");
        assert_eq!(d.default, Focus::Accept);
    }

    #[test]
    fn a_long_name_leaves_the_scrollbar_its_presses() {
        let t = text();
        let mut d = sample();
        d.tree.roots[0]
            .children
            .extend((0..20).map(|k| Node::new(&format!("Folder {k}"), false)));
        d.reopen_edit(vec![0, 1], &"W".repeat(60));
        let lay = d.layout(&t, 11, &ControlMetrics::CLASSIC);
        let sb = lay.tree.scrollbar.expect("the rows overflow");
        let r = lay.edit.expect("the box");
        assert!(r.right() <= lay.tree.body.right(), "{r:?} within the body");
        let arrow = d.hit(&lay, sb.x + 5, sb.y + 5);
        assert!(
            matches!(arrow, Some(Hit::Tree(treeview::Hit::VBar(_)))),
            "{arrow:?}"
        );
    }

    #[test]
    fn a_held_thumb_lights_no_button() {
        let t = text();
        let mut d = sample();
        d.tree.roots[0]
            .children
            .extend((0..20).map(|k| Node::new(&format!("Folder {k}"), false)));
        let lay = d.layout(&t, 11, &ControlMetrics::CLASSIC);
        let sb = lay.tree.scrollbar.expect("the rows overflow");
        let hit = (sb.y..sb.bottom())
            .map(|y| d.hit(&lay, sb.x + 5, y))
            .find(|h| matches!(h, Some(Hit::Tree(treeview::Hit::VBar(BarHit::Thumb(_))))))
            .expect("a thumb");
        d.click(&lay, hit);
        d.motion(&lay, &t, 11, lay.accept.x + 20, lay.accept.y + 10);
        assert_eq!(
            d.buttons.of(Focus::Accept),
            crate::pointer::Interaction::Idle
        );
    }

    #[test]
    fn alt_m_makes_a_new_folder() {
        let mut d = sample();
        let lay = d.layout(&text(), 11, &ControlMetrics::CLASSIC);
        assert_eq!(
            d.key(&lay, form::Key::Access('m')),
            Event::NewFolder(vec![0, 1])
        );
        assert_eq!(d.key(&lay, form::Key::Access('n')), Event::None);
    }

    #[test]
    fn tab_skips_a_greyed_accept_both_ways() {
        let mut d = sample();
        let lay = d.layout(&text(), 11, &ControlMetrics::CLASSIC);
        d.set_enabled(false, true);
        d.focus = Focus::NewFolder;
        d.key(&lay, form::Key::Tab { back: false });
        assert_eq!(d.focus, Focus::Cancel, "Tab from New Folder passes Accept");
        d.key(&lay, form::Key::Tab { back: true });
        assert_eq!(
            d.focus,
            Focus::NewFolder,
            "Shift+Tab from Cancel passes Accept"
        );
    }

    #[test]
    fn greying_the_focused_accept_hands_the_focus_to_the_tree() {
        let mut d = sample();
        assert_eq!(d.focus, Focus::Accept);
        d.set_enabled(false, true);
        assert_eq!(d.focus, Focus::Tree);
        assert!(!d.tree.inactive, "the tree draws its pick as focused");
    }

    #[test]
    fn the_default_ring_follows_a_focused_button() {
        let theme = Theme::default();
        let cs = &theme.colors;
        let t = text();
        let mut d = sample();
        d.focus = Focus::Cancel;
        let lay = d.layout(&t, 11, &ControlMetrics::CLASSIC);
        let mut c = Canvas::new(CLIENT.0 as u32, CLIENT.1 as u32);
        d.draw(&mut c, &t, &classic_paint(cs, 1), &lay, 11);
        assert_eq!(
            c.get(228, 325),
            cs.button_text.argb(),
            "Cancel's ring at x 228"
        );
        assert_eq!(
            c.get(147, 325),
            cs.button_hilight.argb(),
            "Accept's plain raised edge at x 147"
        );
    }

    fn tall(n: usize, last: &str) -> BrowseFolder {
        let mut desk = Node::new("Desktop", true);
        desk.expanded = true;
        desk.children = (1..n)
            .map(|i| Node::new(&format!("Folder {i}"), false))
            .chain([Node::new(last, false)])
            .collect();
        let tv = TreeView::new(vec![desk], Style::Picker);
        BrowseFolder::new("Pick.".into(), "Move".into(), tv)
    }

    fn thumb_at(d: &BrowseFolder, lay: &Layout, sb: Rect, horizontal: bool) -> (i32, i32) {
        let cross = if horizontal {
            sb.y + sb.height / 2
        } else {
            sb.x + sb.width / 2
        };
        let len = if horizontal { sb.width } else { sb.height };
        let start = if horizontal { sb.x } else { sb.y };
        (start..start + len)
            .map(|v| if horizontal { (v, cross) } else { (cross, v) })
            .find(|&(x, y)| {
                matches!(
                    d.hit(lay, x, y),
                    Some(Hit::Tree(treeview::Hit::VBar(BarHit::Thumb(_))))
                        | Some(Hit::Tree(treeview::Hit::HBar(BarHit::Thumb(_))))
                )
            })
            .expect("a thumb on the bar")
    }

    #[test]
    fn the_wheel_over_the_tree_scrolls_it() {
        let mut d = tall(40, "Last");
        let lay = d.layout(&text(), 11, &ControlMetrics::CLASSIC);
        let body = lay.tree.body;
        assert_eq!(d.wheel(&lay, body.x + 5, body.y + 5, 3), Event::Redraw);
        assert_eq!(d.tree.top, 3, "three rows down");
        assert_eq!(d.wheel(&lay, 5, 5, 3), Event::None, "off the tree");
        assert_eq!(d.tree.top, 3);
        d.wheel(&lay, body.x + 5, body.y + 5, -10);
        assert_eq!(d.tree.top, 0, "clamped at the first row");
    }

    #[test]
    fn a_held_vertical_thumb_drags_the_tree_until_release() {
        let t = text();
        let mut d = tall(40, "Last");
        let lay = d.layout(&t, 11, &ControlMetrics::CLASSIC);
        let sb = lay.tree.scrollbar.expect("a vertical bar");
        let (x, y) = thumb_at(&d, &lay, sb, false);
        let hit = d.hit(&lay, x, y);
        assert_eq!(d.click(&lay, hit), Event::None);
        assert_eq!(d.motion(&lay, &t, 11, x, y + 40), Event::Redraw);
        let dragged = d.tree.top;
        assert!(dragged > 0, "the tree follows the thumb");
        assert_eq!(
            d.motion(&lay, &t, 11, x, y + 40),
            Event::None,
            "no move, no redraw"
        );
        d.release();
        assert_eq!(d.motion(&lay, &t, 11, x, y + 80), Event::None);
        assert_eq!(d.tree.top, dragged, "a released thumb stays put");
    }

    #[test]
    fn a_held_horizontal_thumb_drags_the_tree_sideways() {
        let t = text();
        let mut d = tall(3, &"A very long folder name ".repeat(4));
        let lay = d.layout(&t, 11, &ControlMetrics::CLASSIC);
        let hb = lay.tree.hscroll.expect("a horizontal bar");
        let (x, y) = thumb_at(&d, &lay, hb, true);
        let hit = d.hit(&lay, x, y);
        d.click(&lay, hit);
        assert_eq!(d.motion(&lay, &t, 11, x + 60, y), Event::Redraw);
        assert!(d.tree.left > 0, "the rows follow the thumb");
    }

    #[test]
    fn the_prompt_shows_three_lines_and_clips_the_fourth() {
        let theme = Theme::default();
        let cs = &theme.colors;
        let t = text();
        let mut d = sample();
        d.prompt = format!(
            "Pick the folder to move '{}' to, then click Move.",
            "quarterly report draft for the regional sales meeting ".repeat(3)
        );
        assert!(
            text::wrap(&t, &d.prompt, 290, 11).len() > 3,
            "four lines or more"
        );
        let lay = d.layout(&t, 11, &ControlMetrics::CLASSIC);
        let mut c = Canvas::new(CLIENT.0 as u32, CLIENT.1 as u32);
        d.draw(&mut c, &t, &classic_paint(cs, 1), &lay, 11);
        let inked = |y0: i32, y1: i32| {
            (y0..y1).any(|y| (14..300).any(|x| c.get(x, y) == cs.button_text.argb()))
        };
        assert!(inked(40, 51), "third line ink between y 40 and 50");
        assert!(
            !inked(51, 60),
            "nothing between the third line and the tree"
        );
    }

    #[test]
    #[ignore]
    fn render_browsefolder() {
        let theme = Theme::default();
        let t = text();
        let d = sample();
        let lay = d.layout(&t, 11, &ControlMetrics::CLASSIC);
        let mut c = Canvas::new(CLIENT.0 as u32, CLIENT.1 as u32);
        d.draw(&mut c, &t, &classic_paint(&theme.colors, 1), &lay, 11);
        crate::raster::write_preview(&c, "browsefolder-preview.png");
    }
}
