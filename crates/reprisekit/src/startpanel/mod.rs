//! The Glass Start panel as a model: the rows it lists in each mode, the
//! keyboard's walk over them, the pointer's hot spot and its hover delays.

pub mod access;
pub mod layout;
pub mod search;

use crate::raster::Canvas;
use crate::textedit::{EditKey, TextEdit};

pub const ALL_PROGRAMS: &str = "All Programs";
pub const BACK: &str = "Back";
pub const SHUT_DOWN: &str = "Shut down";
pub const SHUT_DOWN_OPTIONS: &str = "Shut down options";
pub const SEARCH_CUE: &str = "Search programs and files";
pub const CLEAR: &str = "Clear";
/// The program the user picture opens.
pub const USER_ACCOUNTS: &str = "User Accounts";
pub const NO_RESULTS: &str = "No items match your search.";
pub const MAX_RESULTS: usize = 8;

/// A pinned or frequent program. `jump` marks a row with a jump list (it
/// draws an arrow and opens the list on hover or Right); `new_pin` a
/// newly pinned one, drawn in its own box.
#[derive(Clone, Debug, PartialEq)]
pub struct ProgramRow {
    pub name: String,
    pub icon: Option<Canvas>,
    pub jump: bool,
    pub new_pin: bool,
}

/// A row of the right column. `picture` is the 48 px picture shown in the
/// frame while the row is lit; `rule_before` draws a rule above the row;
/// `submenu` marks a row that opens a menu of its own.
#[derive(Clone, Debug, PartialEq)]
pub struct PlaceRow {
    pub label: String,
    pub picture: Option<Canvas>,
    pub rule_before: bool,
    pub submenu: bool,
}

/// A row of the program tree. The host lists every folder with its apps
/// after it (`depth` 1); the model shows an app at depth 1 only while the
/// folder above it is open. Apps at depth 0 stand at the top level. A
/// folder's `icon` is the theme's 16 px folder.
#[derive(Clone, Debug, PartialEq)]
pub enum TreeRow {
    Folder {
        name: String,
        open: bool,
        icon: Option<Canvas>,
    },
    App {
        name: String,
        icon: Option<Canvas>,
        depth: u8,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResultRow {
    pub name: String,
    pub icon: Option<Canvas>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResultGroup {
    pub title: String,
    pub rows: Vec<ResultRow>,
}

/// What the panel shows. `Jump(i)` shows the jump list of program row `i`
/// (an index into [`StartPanel::programs`]); that row is drawn lit for as
/// long as the mode lasts, whatever else is lit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Home,
    Programs,
    Search,
    Jump(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Spot {
    Program(usize),
    AllPrograms,
    Tree(usize),
    Result(usize, usize),
    Jump(usize),
    Place(usize),
    Picture,
    Search,
    Clear,
    Shutdown,
    ShutdownArrow,
}

/// A key the host routes to the panel. `Left`, `Right`, `Home` and `End`
/// are the bare keys: in Search mode, with the keyboard in the field, they
/// move the caret; elsewhere they walk the rows. Modified edit keys
/// (Shift+Left, Ctrl+Backspace) come as `Char`. `Menu` is the context
/// menu key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Key {
    Char(EditKey),
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    Enter,
    Escape,
    Tab,
    Menu,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Left,
    Right,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    None,
    Redraw,
    QueryChanged,
    ArmHover(Spot),
    Activate(Spot),
    Context(Spot),
    Close,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StartPanel {
    pub pinned: Vec<ProgramRow>,
    pub frequent: Vec<ProgramRow>,
    pub places: Vec<PlaceRow>,
    pub user_picture: Option<Canvas>,
    pub tree: Vec<TreeRow>,
    pub tree_top: usize,
    pub results: Vec<ResultGroup>,
    pub jump: Vec<ResultRow>,
    pub jump_title: String,
    /// Private so every write goes through `key` or `paste`, which keep
    /// the mode in step with the query.
    edit: TextEdit,
    mode: Mode,
    before_search: Mode,
    focus: Spot,
    hot: Option<Spot>,
    lit: Option<Spot>,
    pictured: Option<usize>,
}

impl Default for StartPanel {
    fn default() -> Self {
        StartPanel::new()
    }
}

#[derive(Clone, Copy)]
enum Step {
    By(i64),
    First,
    Last,
}

impl StartPanel {
    pub fn new() -> Self {
        StartPanel {
            pinned: Vec::new(),
            frequent: Vec::new(),
            places: Vec::new(),
            user_picture: None,
            tree: Vec::new(),
            tree_top: 0,
            results: Vec::new(),
            jump: Vec::new(),
            jump_title: String::new(),
            edit: TextEdit::new("", false),
            mode: Mode::Home,
            before_search: Mode::Home,
            focus: Spot::Search,
            hot: None,
            lit: None,
            pictured: None,
        }
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn query(&self) -> &str {
        &self.edit.text
    }

    pub fn edit(&self) -> &TextEdit {
        &self.edit
    }

    /// The keyboard's stop: `Search` (typing goes into the field) while it
    /// walks the left pane, the results or a jump list; a place, `Shutdown`
    /// or `ShutdownArrow` once it moved there.
    pub fn focus(&self) -> Spot {
        self.focus
    }

    pub fn hot(&self) -> Option<Spot> {
        self.hot
    }

    pub fn lit(&self) -> Option<Spot> {
        self.hot.or_else(|| self.walked())
    }

    /// The place whose picture the picture spot shows in place of the
    /// user picture: the last place lit. Lighting any other spot brings
    /// the user picture back; lighting nothing (a rule, a gap, off the
    /// panel) keeps the place's.
    pub fn picture(&self) -> Option<usize> {
        self.pictured
    }

    pub fn walked(&self) -> Option<Spot> {
        self.lit.or_else(|| {
            (self.mode == Mode::Search)
                .then(|| self.first_result())
                .flatten()
        })
    }

    pub fn programs(&self) -> impl Iterator<Item = &ProgramRow> {
        self.pinned.iter().chain(&self.frequent)
    }

    pub fn program(&self, i: usize) -> Option<&ProgramRow> {
        self.programs().nth(i)
    }

    pub fn tree_rows(&self) -> Vec<usize> {
        let mut open = true;
        let mut rows = Vec::new();
        for (i, r) in self.tree.iter().enumerate() {
            match r {
                TreeRow::Folder { open: o, .. } => {
                    open = *o;
                    rows.push(i);
                }
                TreeRow::App { depth: 0, .. } => {
                    open = true;
                    rows.push(i);
                }
                TreeRow::App { .. } if open => rows.push(i),
                TreeRow::App { .. } => {}
            }
        }
        rows
    }

    pub fn key(&mut self, k: Key) -> Event {
        let in_field = self.mode == Mode::Search && self.focus == Spot::Search;
        match k {
            Key::Char(ek) => self.edit_key(ek),
            Key::Left if in_field => self.edit_key(EditKey::Left {
                select: false,
                word: false,
            }),
            Key::Right if in_field => self.edit_key(EditKey::Right {
                select: false,
                word: false,
            }),
            Key::Home if in_field => self.edit_key(EditKey::Home { select: false }),
            Key::End if in_field => self.edit_key(EditKey::End { select: false }),
            Key::Up => self.walk(Step::By(-1)),
            Key::Down => self.walk(Step::By(1)),
            Key::Home => self.walk(Step::First),
            Key::End => self.walk(Step::Last),
            Key::Left => self.left(),
            Key::Right => self.right(),
            Key::Tab => self.tab(),
            Key::Enter => self.enter(),
            Key::Escape => self.escape(),
            Key::Menu => match self.lit() {
                Some(s) if self.has_context(s) => Event::Context(s),
                _ => Event::None,
            },
        }
    }

    pub fn paste(&mut self, s: &str) -> Event {
        let before = self.edit.text.clone();
        let moved = self.edit.paste(s, &[]);
        self.edited(&before, moved)
    }

    /// Text dropped on the search field: its first line, as a paste takes it, becomes the
    /// query.
    pub fn drop_query(&mut self, s: &str) -> Event {
        let mut edit = TextEdit::new("", false);
        if !edit.paste(s, &[]) {
            return Event::None;
        }
        let before = std::mem::replace(&mut self.edit, edit);
        self.edited(&before.text, true)
    }

    pub fn pointer(&mut self, at: Option<Spot>) -> Event {
        if at == self.hot {
            return Event::None;
        }
        self.hot = at;
        self.follow_lit();
        match at {
            Some(s @ Spot::Program(i)) => match self.mode {
                Mode::Jump(j) if j == i => Event::Redraw,
                Mode::Jump(_) => Event::ArmHover(s),
                _ if self.program(i).is_some_and(|r| r.jump) => Event::ArmHover(s),
                _ => Event::Redraw,
            },
            Some(Spot::AllPrograms) if self.mode != Mode::Programs => {
                Event::ArmHover(Spot::AllPrograms)
            }
            Some(Spot::ShutdownArrow) => Event::ArmHover(Spot::ShutdownArrow),
            _ => Event::Redraw,
        }
    }

    pub fn press(&mut self, at: Spot, button: Button) -> Event {
        match button {
            Button::Left => self.activate(at),
            Button::Right if self.has_context(at) => Event::Context(at),
            Button::Right => Event::None,
        }
    }

    /// The hover delay armed for `at` ran out. A row with a jump list
    /// opens it (another row closes an open one), All Programs opens the
    /// tree, the arrow opens the Shut down options. A fire for a spot the
    /// pointer has left does nothing.
    pub fn hover_fired(&mut self, at: Spot) -> Event {
        if self.hot != Some(at) {
            return Event::None;
        }
        match at {
            Spot::Program(i) => {
                let jump = self.program(i).is_some_and(|r| r.jump);
                match self.mode {
                    m if jump && m != Mode::Jump(i) => {
                        self.set_mode(Mode::Jump(i));
                        Event::Redraw
                    }
                    Mode::Jump(_) if !jump => {
                        self.set_mode(Mode::Home);
                        Event::Redraw
                    }
                    _ => Event::None,
                }
            }
            Spot::AllPrograms if self.mode != Mode::Programs => {
                self.set_mode(Mode::Programs);
                Event::Redraw
            }
            Spot::ShutdownArrow => Event::Activate(Spot::ShutdownArrow),
            _ => Event::None,
        }
    }

    /// Switch modes. The keyboard goes back to the field with nothing lit;
    /// the pointer's row is dropped (the new view may not have it) until
    /// the pointer next moves; entering Programs scrolls the tree to its top.
    pub fn set_mode(&mut self, m: Mode) {
        if m == Mode::Programs && self.mode != Mode::Programs {
            self.tree_top = 0;
        }
        self.mode = m;
        self.focus = Spot::Search;
        self.lit = None;
        self.hot = None;
    }

    pub fn toggle_folder(&mut self, tree_row: usize) {
        let Some(TreeRow::Folder { open, .. }) = self.tree.get_mut(tree_row) else {
            return;
        };
        *open = !*open;
        let shown = self.tree_rows();
        if matches!(self.lit, Some(Spot::Tree(i)) if !shown.contains(&i)) {
            self.lit = Some(Spot::Tree(tree_row));
        }
        if matches!(self.hot, Some(Spot::Tree(i)) if !shown.contains(&i)) {
            self.hot = None;
        }
        self.scroll_tree(0);
    }

    pub fn scroll_tree(&mut self, rows: i64) -> Event {
        let max = self
            .tree_rows()
            .len()
            .saturating_sub(layout::tree_capacity(self));
        let top = (self.tree_top as i64 + rows).clamp(0, max as i64) as usize;
        if top == self.tree_top {
            return Event::None;
        }
        self.tree_top = top;
        // Another row is under the pointer now; its next motion finds it.
        self.hot = None;
        Event::Redraw
    }

    /// Drops the jump list rows `gone` marks. A lit row that goes passes the light to the row
    /// sliding into its place, else the one above; an emptied list goes back to Home, lighting
    /// its program row if a jump row was lit.
    pub fn drop_jump_rows(&mut self, gone: &[bool]) {
        let goes = |j: usize| gone.get(j).copied().unwrap_or(false);
        let kept = |k: usize| (0..k).filter(|&j| !goes(j)).count();
        let mut j = 0;
        self.jump.retain(|_| {
            j += 1;
            !goes(j - 1)
        });
        if matches!(self.hot, Some(Spot::Jump(_))) {
            self.hot = None;
        }
        let Some(Spot::Jump(k)) = self.lit else {
            if self.jump.is_empty()
                && let Mode::Jump(_) = self.mode
            {
                self.set_mode(Mode::Home);
            }
            return;
        };
        match (self.jump.len(), self.mode) {
            (0, Mode::Jump(i)) => {
                self.set_mode(Mode::Home);
                self.light(Spot::Program(i));
            }
            (0, _) => self.lit = None,
            (n, _) => self.lit = Some(Spot::Jump(kept(k).min(n - 1))),
        }
    }

    pub fn set_results(&mut self, groups: Vec<ResultGroup>) {
        self.results = groups
            .into_iter()
            .map(|mut g| {
                g.rows.truncate(MAX_RESULTS);
                g
            })
            .collect();
        let gone = |s: Option<Spot>, r: &[ResultGroup]| matches!(s, Some(Spot::Result(g, i)) if r.get(g).is_none_or(|g| i >= g.rows.len()));
        if gone(self.lit, &self.results) {
            self.lit = None;
        }
        if gone(self.hot, &self.results) {
            self.hot = None;
        }
    }

    fn first_result(&self) -> Option<Spot> {
        self.results
            .iter()
            .position(|g| !g.rows.is_empty())
            .map(|g| Spot::Result(g, 0))
    }

    fn has_context(&self, s: Spot) -> bool {
        match s {
            Spot::Program(_) | Spot::Result(..) => true,
            Spot::Tree(i) => matches!(self.tree.get(i), Some(TreeRow::App { .. })),
            _ => false,
        }
    }

    fn edit_key(&mut self, ek: EditKey) -> Event {
        let before = self.edit.text.clone();
        let moved = self.edit.key(ek);
        self.edited(&before, moved)
    }

    fn edited(&mut self, before: &str, moved: bool) -> Event {
        if self.edit.text == before {
            return if moved { Event::Redraw } else { Event::None };
        }
        self.focus = Spot::Search;
        self.lit = None;
        let empty = self.edit.text.is_empty();
        if !empty && self.mode != Mode::Search {
            self.before_search = match self.mode {
                Mode::Programs => Mode::Programs,
                _ => Mode::Home,
            };
            self.set_mode(Mode::Search);
        } else if empty && self.mode == Mode::Search {
            self.mode = self.before_search;
            self.hot = None;
        }
        Event::QueryChanged
    }

    fn clear_query(&mut self) -> Event {
        let before = std::mem::replace(&mut self.edit, TextEdit::new("", false));
        self.edited(&before.text, true)
    }

    fn follow_lit(&mut self) {
        match self.lit() {
            Some(Spot::Place(i)) => self.pictured = Some(i),
            Some(_) => self.pictured = None,
            None => {}
        }
    }

    fn light(&mut self, s: Spot) {
        self.hot = None;
        self.lit = Some(s);
        self.follow_lit();
        self.focus = match s {
            Spot::Place(_) | Spot::Shutdown | Spot::ShutdownArrow => s,
            _ => Spot::Search,
        };
        if let Spot::Tree(i) = s {
            self.scroll_to(i);
        }
    }

    fn scroll_to(&mut self, i: usize) {
        let Some(at) = self.tree_rows().iter().position(|&r| r == i) else {
            return;
        };
        let cap = layout::tree_capacity(self).max(1);
        if at < self.tree_top {
            self.tree_top = at;
        } else if at >= self.tree_top + cap {
            self.tree_top = at + 1 - cap;
        }
    }

    fn left_column(&self) -> Vec<Spot> {
        match self.mode {
            Mode::Search => self
                .results
                .iter()
                .enumerate()
                .flat_map(|(g, grp)| (0..grp.rows.len()).map(move |r| Spot::Result(g, r)))
                .collect(),
            Mode::Programs => {
                let mut col: Vec<Spot> = self.tree_rows().into_iter().map(Spot::Tree).collect();
                col.push(Spot::AllPrograms);
                col
            }
            Mode::Home | Mode::Jump(_) => {
                let mut col: Vec<Spot> = (0..layout::programs_shown(self))
                    .map(Spot::Program)
                    .collect();
                col.push(Spot::AllPrograms);
                col
            }
        }
    }

    fn right_column(&self) -> Vec<Spot> {
        let places = matches!(self.mode, Mode::Home | Mode::Programs);
        let mut col: Vec<Spot> = if places {
            (0..self.places.len()).map(Spot::Place).collect()
        } else {
            Vec::new()
        };
        col.push(Spot::Shutdown);
        col
    }

    fn walk(&mut self, step: Step) -> Event {
        let from = self.lit();
        let col = match from {
            Some(Spot::Place(_) | Spot::Shutdown | Spot::ShutdownArrow) => self.right_column(),
            Some(Spot::Jump(_)) => (0..layout::jumps_shown(self)).map(Spot::Jump).collect(),
            _ => self.left_column(),
        };
        if col.is_empty() {
            return Event::None;
        }
        let from = from.map(|s| match s {
            Spot::ShutdownArrow => Spot::Shutdown,
            s => s,
        });
        let at = from.and_then(|s| col.iter().position(|&c| c == s));
        let last = col.len() - 1;
        let to = match (step, at) {
            (Step::First, _) | (Step::By(1..), None) => 0,
            (Step::Last, _) | (Step::By(..=0), None) => last,
            (Step::By(d), Some(i)) => (i as i64 + d).clamp(0, last as i64) as usize,
        };
        // Walking the programs closes a jump list the pointer opened.
        if matches!(self.mode, Mode::Jump(_)) && matches!(col[to], Spot::Program(_)) {
            self.set_mode(Mode::Home);
        }
        self.light(col[to]);
        Event::Redraw
    }

    fn left(&mut self) -> Event {
        match self.lit() {
            Some(Spot::ShutdownArrow) => self.light(Spot::Shutdown),
            Some(Spot::Place(_) | Spot::Shutdown) => match self.left_column().first() {
                Some(&s) => self.light(s),
                None => return Event::None,
            },
            Some(Spot::Jump(_)) => {
                let Mode::Jump(i) = self.mode else {
                    return Event::None;
                };
                self.set_mode(Mode::Home);
                self.light(Spot::Program(i));
            }
            Some(Spot::Tree(i)) => match self.tree.get(i) {
                Some(TreeRow::Folder { open: true, .. }) => self.toggle_folder(i),
                Some(TreeRow::App { depth: 1.., .. }) => {
                    match self.tree[..i]
                        .iter()
                        .rposition(|r| matches!(r, TreeRow::Folder { .. }))
                    {
                        Some(f) => self.light(Spot::Tree(f)),
                        None => return Event::None,
                    }
                }
                _ => return Event::None,
            },
            _ => return Event::None,
        }
        Event::Redraw
    }

    fn right(&mut self) -> Event {
        match self.lit() {
            Some(Spot::Program(i)) if self.program(i).is_some_and(|r| r.jump) => {
                self.set_mode(Mode::Jump(i));
                self.light(Spot::Jump(0));
            }
            Some(Spot::AllPrograms) if self.mode != Mode::Programs => {
                self.set_mode(Mode::Programs);
                if let Some(&first) = self.tree_rows().first() {
                    self.light(Spot::Tree(first));
                }
            }
            Some(Spot::Tree(i)) => match self.tree.get(i) {
                Some(TreeRow::Folder { open: false, .. }) => self.toggle_folder(i),
                _ => return Event::None,
            },
            Some(Spot::Place(i)) if self.places.get(i).is_some_and(|p| p.submenu) => {
                return Event::Activate(Spot::Place(i));
            }
            Some(Spot::Shutdown) => self.light(Spot::ShutdownArrow),
            Some(Spot::ShutdownArrow) => return Event::Activate(Spot::ShutdownArrow),
            Some(Spot::Place(_) | Spot::Jump(_) | Spot::AllPrograms | Spot::Result(..)) => {
                return Event::None;
            }
            _ => match self.right_column().first() {
                Some(&s) => self.light(s),
                None => return Event::None,
            },
        }
        Event::Redraw
    }

    fn tab(&mut self) -> Event {
        let next = match self.focus {
            Spot::Search => self.right_column()[0],
            Spot::Place(_) => Spot::Shutdown,
            Spot::Shutdown => Spot::ShutdownArrow,
            _ => Spot::Search,
        };
        if next == Spot::Search {
            self.hot = None;
            self.lit = None;
            self.focus = Spot::Search;
        } else {
            self.light(next);
        }
        Event::Redraw
    }

    fn enter(&mut self) -> Event {
        let target = match self.mode {
            Mode::Search => match self.lit() {
                Some(s @ Spot::Result(..)) => Some(s),
                _ if self.focus != Spot::Search => Some(self.focus),
                _ => self.first_result(),
            },
            _ => self.lit().or(Some(self.focus)),
        };
        match target {
            Some(Spot::Search) | None => Event::None,
            Some(s) => self.activate(s),
        }
    }

    fn escape(&mut self) -> Event {
        match self.mode {
            Mode::Search if !self.query().is_empty() => self.clear_query(),
            Mode::Search => {
                self.set_mode(self.before_search);
                Event::Redraw
            }
            Mode::Programs => {
                self.set_mode(Mode::Home);
                self.light(Spot::AllPrograms);
                Event::Redraw
            }
            Mode::Jump(i) => {
                self.set_mode(Mode::Home);
                self.light(Spot::Program(i));
                Event::Redraw
            }
            Mode::Home => Event::Close,
        }
    }

    fn activate(&mut self, s: Spot) -> Event {
        match s {
            Spot::AllPrograms => {
                let to = if self.mode == Mode::Programs {
                    Mode::Home
                } else {
                    Mode::Programs
                };
                self.set_mode(to);
                Event::Redraw
            }
            Spot::Tree(i) if matches!(self.tree.get(i), Some(TreeRow::Folder { .. })) => {
                self.toggle_folder(i);
                Event::Redraw
            }
            Spot::Search => {
                self.focus = Spot::Search;
                self.lit = None;
                Event::Redraw
            }
            Spot::Clear => self.clear_query(),
            Spot::Jump(k) if k >= self.jump.len() => Event::None,
            Spot::Result(g, r) if self.results.get(g).is_none_or(|g| r >= g.rows.len()) => {
                Event::None
            }
            _ => Event::Activate(s),
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn prow(name: &str, jump: bool) -> ProgramRow {
        ProgramRow {
            name: name.into(),
            icon: None,
            jump,
            new_pin: false,
        }
    }

    pub(crate) fn place(label: &str) -> PlaceRow {
        PlaceRow {
            label: label.into(),
            picture: None,
            rule_before: false,
            submenu: false,
        }
    }

    pub(crate) fn row(name: &str) -> ResultRow {
        ResultRow {
            name: name.into(),
            icon: None,
        }
    }

    fn typed(s: &str) -> Key {
        Key::Char(EditKey::Insert(s.into()))
    }

    fn folder(name: &str) -> TreeRow {
        TreeRow::Folder {
            name: name.into(),
            open: false,
            icon: None,
        }
    }

    fn app(name: &str, depth: u8) -> TreeRow {
        TreeRow::App {
            name: name.into(),
            icon: None,
            depth,
        }
    }

    #[test]
    fn typing_searches_and_escape_unwinds() {
        let mut p = StartPanel::new();
        assert_eq!(p.mode(), Mode::Home);
        assert_eq!(p.focus(), Spot::Search);
        assert_eq!(p.key(typed("no")), Event::QueryChanged);
        assert_eq!(p.mode(), Mode::Search);
        assert_eq!(p.key(Key::Escape), Event::QueryChanged);
        assert_eq!((p.mode(), p.query()), (Mode::Home, ""));
        assert_eq!(p.key(Key::Escape), Event::Close);
    }

    #[test]
    fn an_emptied_query_goes_back_to_the_tree() {
        let mut p = StartPanel::new();
        p.set_mode(Mode::Programs);
        p.key(typed("n"));
        assert_eq!(p.mode(), Mode::Search);
        assert_eq!(
            p.key(Key::Char(EditKey::Backspace { word: false })),
            Event::QueryChanged
        );
        assert_eq!(p.mode(), Mode::Programs);
        assert_eq!(p.paste("x"), Event::QueryChanged);
        assert_eq!((p.mode(), p.query()), (Mode::Search, "x"));
        assert_eq!(
            p.press(Spot::Clear, Button::Left),
            Event::QueryChanged,
            "the clear button empties the field"
        );
        assert_eq!((p.mode(), p.query()), (Mode::Programs, ""));
    }

    #[test]
    fn escape_from_the_tree_or_a_jump_list_goes_home() {
        let mut p = StartPanel::new();
        p.frequent = vec![prow("Notepad", true)];
        p.set_mode(Mode::Programs);
        assert_eq!(p.key(Key::Escape), Event::Redraw);
        assert_eq!(p.mode(), Mode::Home);
        assert_eq!(
            p.lit(),
            Some(Spot::AllPrograms),
            "Back lands on All Programs"
        );
        p.set_mode(Mode::Jump(0));
        assert_eq!(p.key(Key::Escape), Event::Redraw);
        assert_eq!((p.mode(), p.lit()), (Mode::Home, Some(Spot::Program(0))));
        assert_eq!(p.key(Key::Escape), Event::Close);
    }

    #[test]
    fn enter_in_search_runs_the_first_result() {
        let mut p = StartPanel::new();
        p.key(typed("pa"));
        p.set_results(vec![ResultGroup {
            title: "Programs".into(),
            rows: vec![row("Paint"), row("Pager")],
        }]);
        assert_eq!(p.lit(), Some(Spot::Result(0, 0)), "the first result is lit");
        assert_eq!(p.key(Key::Enter), Event::Activate(Spot::Result(0, 0)));
    }

    #[test]
    fn down_walks_the_results_across_groups_and_enter_runs_the_lit_one() {
        let mut p = StartPanel::new();
        p.key(typed("no"));
        p.set_results(vec![
            ResultGroup {
                title: "Programs (1)".into(),
                rows: vec![row("Notepad")],
            },
            ResultGroup {
                title: "Files (2)".into(),
                rows: vec![row("notes.txt"), row("north.png")],
            },
        ]);
        p.key(Key::Down);
        assert_eq!(p.lit(), Some(Spot::Result(1, 0)));
        assert_eq!(p.key(Key::Enter), Event::Activate(Spot::Result(1, 0)));
        p.set_results(vec![ResultGroup {
            title: "Programs (1)".into(),
            rows: vec![row("Notepad")],
        }]);
        assert_eq!(p.lit(), Some(Spot::Result(0, 0)), "a gone result gives way");
    }

    #[test]
    fn a_group_keeps_eight_results_at_most() {
        let mut p = StartPanel::new();
        p.set_results(vec![ResultGroup {
            title: "Files".into(),
            rows: (0..12).map(|k| row(&format!("f{k}"))).collect(),
        }]);
        assert_eq!(p.results[0].rows.len(), MAX_RESULTS);
    }

    #[test]
    fn a_drop_on_the_search_field_replaces_the_query_and_searches() {
        let mut p = StartPanel::new();
        p.paste("old");
        assert_eq!(p.drop_query("notes\nmore"), Event::QueryChanged);
        assert_eq!((p.mode(), p.query()), (Mode::Search, "notes"));
        assert_eq!(p.edit().caret(), 5);
        assert_eq!(p.drop_query("\u{7}"), Event::None, "nothing to drop");
        assert_eq!(p.query(), "notes");
    }

    #[test]
    fn in_search_the_bare_arrows_move_the_caret() {
        let mut p = StartPanel::new();
        p.key(typed("ab"));
        assert_eq!(p.key(Key::Left), Event::Redraw);
        assert_eq!(p.edit().caret(), 1);
        assert_eq!(p.key(Key::Home), Event::Redraw);
        assert_eq!(p.edit().caret(), 0);
        assert_eq!(p.mode(), Mode::Search);
    }

    #[test]
    fn hovering_all_programs_opens_the_tree_after_the_delay() {
        let mut p = StartPanel::new();
        assert_eq!(
            p.pointer(Some(Spot::AllPrograms)),
            Event::ArmHover(Spot::AllPrograms)
        );
        assert_eq!(p.hover_fired(Spot::AllPrograms), Event::Redraw);
        assert_eq!(p.mode(), Mode::Programs);
        assert_eq!(
            p.pointer(Some(Spot::Search)),
            Event::Redraw,
            "Back arms nothing"
        );
        assert_eq!(p.pointer(Some(Spot::AllPrograms)), Event::Redraw);
    }

    #[test]
    fn typing_over_a_hot_program_lights_the_first_result() {
        let mut p = StartPanel::new();
        p.frequent = vec![prow("Notepad", false)];
        p.pointer(Some(Spot::Program(0)));
        p.key(typed("no"));
        p.set_results(vec![ResultGroup {
            title: "Programs (1)".into(),
            rows: vec![row("Notepad")],
        }]);
        assert_eq!(p.lit(), Some(Spot::Result(0, 0)));
        p.pointer(Some(Spot::Result(0, 0)));
        p.key(Key::Escape);
        assert_eq!(
            (p.mode(), p.hot()),
            (Mode::Home, None),
            "the result is gone"
        );
    }

    #[test]
    fn a_scroll_drops_the_hot_tree_row() {
        let mut p = StartPanel::new();
        p.tree = (0..30).map(|k| app(&format!("App {k}"), 0)).collect();
        p.set_mode(Mode::Programs);
        p.pointer(Some(Spot::Tree(4)));
        assert_eq!(p.scroll_tree(3), Event::Redraw);
        assert_eq!(p.hot(), None);
    }

    #[test]
    fn a_stale_hover_does_nothing() {
        let mut p = StartPanel::new();
        p.pointer(Some(Spot::AllPrograms));
        p.pointer(Some(Spot::Place(0)));
        assert_eq!(p.hover_fired(Spot::AllPrograms), Event::None);
        assert_eq!(p.mode(), Mode::Home);
    }

    #[test]
    fn hovering_a_row_with_a_jump_list_opens_it_and_keeps_it_over_the_list() {
        let mut p = StartPanel::new();
        p.frequent = vec![prow("Notepad", true), prow("Paint", false)];
        assert_eq!(
            p.pointer(Some(Spot::Program(1))),
            Event::Redraw,
            "no jump list, nothing to arm"
        );
        assert_eq!(
            p.pointer(Some(Spot::Program(0))),
            Event::ArmHover(Spot::Program(0))
        );
        assert_eq!(p.hover_fired(Spot::Program(0)), Event::Redraw);
        assert_eq!(p.mode(), Mode::Jump(0));
        // On the way to the list the pointer crosses another row; the
        // list stays open once it arrives.
        assert_eq!(
            p.pointer(Some(Spot::Program(1))),
            Event::ArmHover(Spot::Program(1))
        );
        assert_eq!(p.pointer(Some(Spot::Jump(2))), Event::Redraw);
        assert_eq!(p.hover_fired(Spot::Program(1)), Event::None);
        assert_eq!(p.mode(), Mode::Jump(0));
        assert_eq!(p.lit(), Some(Spot::Jump(2)));
        p.pointer(Some(Spot::Program(1)));
        assert_eq!(p.hover_fired(Spot::Program(1)), Event::Redraw);
        assert_eq!(p.mode(), Mode::Home);
    }

    #[test]
    fn hovering_the_arrow_opens_the_shut_down_options() {
        let mut p = StartPanel::new();
        assert_eq!(
            p.pointer(Some(Spot::ShutdownArrow)),
            Event::ArmHover(Spot::ShutdownArrow)
        );
        assert_eq!(
            p.hover_fired(Spot::ShutdownArrow),
            Event::Activate(Spot::ShutdownArrow)
        );
        assert_eq!(p.pointer(Some(Spot::ShutdownArrow)), Event::None, "no move");
    }

    #[test]
    fn right_on_a_program_with_recent_files_opens_its_jump_list() {
        let mut p = StartPanel::new();
        p.frequent = vec![prow("Notepad", true)];
        p.key(Key::Down);
        assert_eq!(p.lit(), Some(Spot::Program(0)));
        assert_eq!(p.key(Key::Right), Event::Redraw);
        assert_eq!(p.mode(), Mode::Jump(0));
        assert_eq!(p.lit(), Some(Spot::Jump(0)));
        assert_eq!(p.key(Key::Left), Event::Redraw);
        assert_eq!((p.mode(), p.lit()), (Mode::Home, Some(Spot::Program(0))));
    }

    #[test]
    fn dropped_jump_rows_pass_the_light_on_and_an_emptied_list_goes_home() {
        let row = |name: &str| ResultRow {
            name: name.into(),
            icon: None,
        };
        let mut p = StartPanel::new();
        p.frequent = vec![prow("Notepad", true)];
        p.jump = vec![row("a.txt"), row("b.txt"), row("c.txt")];
        p.key(Key::Down);
        p.key(Key::Right);
        p.key(Key::Down);
        assert_eq!(p.lit(), Some(Spot::Jump(1)));
        p.drop_jump_rows(&[false, true, false]);
        assert_eq!(p.jump, [row("a.txt"), row("c.txt")]);
        assert_eq!(p.lit(), Some(Spot::Jump(1)), "c.txt slid under the light");
        p.drop_jump_rows(&[false, true]);
        assert_eq!(p.lit(), Some(Spot::Jump(0)), "no row below: the one above");
        p.drop_jump_rows(&[true]);
        assert_eq!((p.mode(), p.lit()), (Mode::Home, Some(Spot::Program(0))));
    }

    #[test]
    fn up_and_down_walk_the_programs_then_all_programs_with_typing_kept_in_the_field() {
        let mut p = StartPanel::new();
        p.pinned = vec![prow("Explorer", false)];
        p.frequent = vec![prow("Notepad", false)];
        assert_eq!(p.key(Key::Up), Event::Redraw);
        assert_eq!(p.lit(), Some(Spot::AllPrograms), "Up from the field");
        p.key(Key::Home);
        assert_eq!(p.lit(), Some(Spot::Program(0)));
        p.key(Key::Down);
        assert_eq!(p.lit(), Some(Spot::Program(1)));
        assert_eq!(p.focus(), Spot::Search);
        assert_eq!(p.key(Key::Enter), Event::Activate(Spot::Program(1)));
        p.key(Key::Down);
        p.key(Key::Down);
        assert_eq!(
            p.lit(),
            Some(Spot::AllPrograms),
            "the walk stops at the end"
        );
        assert_eq!(p.key(Key::Enter), Event::Redraw);
        assert_eq!(p.mode(), Mode::Programs);
    }

    #[test]
    fn right_on_all_programs_enters_the_tree_on_its_first_row() {
        let mut p = StartPanel::new();
        p.tree = vec![folder("Graphics"), app("Paint", 1)];
        p.key(Key::Up);
        assert_eq!(p.key(Key::Right), Event::Redraw);
        assert_eq!((p.mode(), p.lit()), (Mode::Programs, Some(Spot::Tree(0))));
    }

    #[test]
    fn the_picture_keeps_the_last_place_until_another_spot_lights() {
        let mut p = StartPanel::new();
        p.places = vec![place("Documents"), place("Music")];
        assert_eq!(p.picture(), None, "the user picture");
        p.pointer(Some(Spot::Place(1)));
        assert_eq!(p.picture(), Some(1));
        p.pointer(None);
        assert_eq!(p.picture(), Some(1), "a rule or a gap keeps Music's");
        p.pointer(Some(Spot::Shutdown));
        assert_eq!(p.picture(), None, "Shut down brings the user picture back");
        p.key(Key::Tab);
        assert_eq!(p.picture(), Some(0), "the keyboard's place too");
        p.pointer(Some(Spot::Program(0)));
        assert_eq!(p.picture(), None, "a program row");
    }

    #[test]
    fn a_click_on_the_user_picture_activates_it() {
        let mut p = StartPanel::new();
        let l = layout::layout(&p);
        let r = l.rect(Spot::Picture).expect("Home shows the picture");
        let at = layout::hit(&l, r.x + r.width / 2, r.y + r.height / 2);
        assert_eq!(at, Some(Spot::Picture));
        assert_eq!(
            p.press(Spot::Picture, Button::Left),
            Event::Activate(Spot::Picture)
        );
        assert_eq!(p.press(Spot::Picture, Button::Right), Event::None);
    }

    #[test]
    fn tab_goes_to_the_places_then_shut_down_its_arrow_and_back() {
        let mut p = StartPanel::new();
        p.places = vec![place("Documents"), place("Music")];
        p.key(Key::Tab);
        assert_eq!((p.focus(), p.lit()), (Spot::Place(0), Some(Spot::Place(0))));
        p.key(Key::Down);
        assert_eq!(p.focus(), Spot::Place(1));
        p.key(Key::Tab);
        assert_eq!(p.focus(), Spot::Shutdown);
        p.key(Key::Tab);
        assert_eq!(p.focus(), Spot::ShutdownArrow);
        assert_eq!(
            p.key(Key::Enter),
            Event::Activate(Spot::ShutdownArrow),
            "Enter opens the options"
        );
        p.key(Key::Tab);
        assert_eq!((p.focus(), p.lit()), (Spot::Search, None));
    }

    #[test]
    fn right_from_the_field_walks_to_the_places_and_shut_down() {
        let mut p = StartPanel::new();
        p.places = vec![place("Documents")];
        p.key(Key::Right);
        assert_eq!(p.focus(), Spot::Place(0));
        p.key(Key::Down);
        assert_eq!(p.focus(), Spot::Shutdown);
        p.key(Key::Right);
        assert_eq!(p.focus(), Spot::ShutdownArrow);
        p.key(Key::Left);
        assert_eq!(p.focus(), Spot::Shutdown);
        assert_eq!(p.key(Key::Enter), Event::Activate(Spot::Shutdown));
        p.key(Key::Left);
        assert_eq!(p.focus(), Spot::Search, "back to the left pane");
        assert_eq!(p.lit(), Some(Spot::AllPrograms));
    }

    #[test]
    fn typing_on_a_place_takes_the_keyboard_back_to_the_field() {
        let mut p = StartPanel::new();
        p.places = vec![place("Documents")];
        p.key(Key::Tab);
        assert_eq!(p.key(typed("n")), Event::QueryChanged);
        assert_eq!(p.focus(), Spot::Search);
    }

    #[test]
    fn a_right_click_opens_a_context_menu_on_rows_only() {
        let mut p = StartPanel::new();
        p.frequent = vec![prow("Notepad", false)];
        p.places = vec![place("Documents")];
        p.tree = vec![folder("Graphics"), app("Paint", 1)];
        for s in [Spot::Program(0), Spot::Tree(1), Spot::Result(0, 0)] {
            assert_eq!(p.press(s, Button::Right), Event::Context(s));
        }
        for s in [
            Spot::Place(0),
            Spot::Tree(0),
            Spot::AllPrograms,
            Spot::Shutdown,
            Spot::Jump(0),
        ] {
            assert_eq!(p.press(s, Button::Right), Event::None, "{s:?}");
        }
        p.key(Key::Down);
        assert_eq!(p.key(Key::Menu), Event::Context(Spot::Program(0)));
        p.lit = Some(Spot::Place(0));
        assert_eq!(p.key(Key::Menu), Event::None, "a place has no menu");
    }

    #[test]
    fn a_click_runs_rows_and_toggles_all_programs_and_folders_in_place() {
        let mut p = StartPanel::new();
        p.frequent = vec![prow("Notepad", true)];
        p.tree = vec![folder("Graphics"), app("Paint", 1)];
        assert_eq!(
            p.press(Spot::Program(0), Button::Left),
            Event::Activate(Spot::Program(0))
        );
        assert_eq!(p.press(Spot::AllPrograms, Button::Left), Event::Redraw);
        assert_eq!(p.mode(), Mode::Programs);
        assert_eq!(p.press(Spot::Tree(0), Button::Left), Event::Redraw);
        assert_eq!(p.tree_rows(), [0, 1], "the folder opened");
        assert_eq!(
            p.press(Spot::Tree(1), Button::Left),
            Event::Activate(Spot::Tree(1))
        );
        assert_eq!(p.press(Spot::AllPrograms, Button::Left), Event::Redraw);
        assert_eq!(p.mode(), Mode::Home, "Back");
        assert_eq!(
            p.press(Spot::Jump(0), Button::Left),
            Event::None,
            "no jump row"
        );
    }

    #[test]
    fn closed_folders_hide_their_apps_and_a_hidden_lit_row_gives_way_to_its_folder() {
        let mut p = StartPanel::new();
        p.tree = vec![
            app("Terminal", 0),
            folder("Graphics"),
            app("Paint", 1),
            app("Gimp", 1),
            folder("Office"),
            app("Writer", 1),
        ];
        assert_eq!(p.tree_rows(), [0, 1, 4]);
        p.toggle_folder(1);
        assert_eq!(p.tree_rows(), [0, 1, 2, 3, 4]);
        p.set_mode(Mode::Programs);
        p.key(Key::Down);
        p.key(Key::Down);
        p.key(Key::Down);
        assert_eq!(p.lit(), Some(Spot::Tree(2)));
        assert_eq!(p.key(Key::Left), Event::Redraw);
        assert_eq!(
            p.lit(),
            Some(Spot::Tree(1)),
            "Left from an app to its folder"
        );
        p.key(Key::Down);
        p.toggle_folder(1);
        assert_eq!(p.tree_rows(), [0, 1, 4]);
        assert_eq!(p.lit(), Some(Spot::Tree(1)));
        p.key(Key::Right);
        assert_eq!(
            p.tree_rows(),
            [0, 1, 2, 3, 4],
            "Right opens a closed folder"
        );
        p.key(Key::Left);
        assert_eq!(p.tree_rows(), [0, 1, 4], "Left closes an open one");
    }

    #[test]
    fn the_tree_scrolls_to_keep_the_lit_row_shown() {
        let mut p = StartPanel::new();
        p.tree = (0..30).map(|k| app(&format!("App {k}"), 0)).collect();
        p.set_mode(Mode::Programs);
        let cap = layout::tree_capacity(&p);
        assert!(cap < 30);
        p.key(Key::Up);
        p.key(Key::Up);
        assert_eq!(p.lit(), Some(Spot::Tree(29)));
        assert_eq!(p.tree_top, 30 - cap, "the last row sits at the bottom");
        p.key(Key::Home);
        assert_eq!(p.tree_top, 0);
        assert_eq!(p.scroll_tree(3), Event::Redraw);
        assert_eq!(p.scroll_tree(100), Event::Redraw);
        assert_eq!(p.tree_top, 30 - cap, "scrolling stops at the last row");
        assert_eq!(p.scroll_tree(1), Event::None);
    }
}
