//! Find in a tab's scrollback: literal matches from `alacritty_terminal`'s regex search, the
//! modeless dialog, and the cells to highlight.

use alacritty_terminal::event::EventListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Boundary, Column, Direction, Line, Point, Side};
use alacritty_terminal::term::Term;
use alacritty_terminal::term::search::{Match, RegexIter, RegexSearch};
use repriseapp::keys::{form_clipboard, form_input};
use repriseapp::oneshot::{self, MsgBox};
use repriseapp::{Ctx, Event, Reply};
use reprisekit::access::Tree;
use reprisekit::controls::finddlg::{self, FindFields};
use reprisekit::controls::form::{self, Control, Form};
use reprisekit::controls::msgbox::Fields;
use reprisekit::look::Paint;
use reprisekit::raster::Canvas;

use crate::Terminal;

/// What the Find dialog holds.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Query {
    pub text: String,
    pub match_case: bool,
    pub up: bool,
}

/// The matches in view and the current one, for [`crate::snapshot::fill_marked`].
#[derive(Default)]
pub struct Marks {
    /// In grid order.
    pub all: Vec<Match>,
    pub current: Option<Match>,
}

impl Marks {
    pub fn is_match(&self, p: Point) -> bool {
        let i = self.all.partition_point(|m| *m.end() < p);
        self.all.get(i).is_some_and(|m| m.contains(&p))
    }

    pub fn is_current(&self, p: Point) -> bool {
        self.current.as_ref().is_some_and(|m| m.contains(&p))
    }
}

/// A query's compiled search and the match last stepped to.
pub struct Search {
    query: (String, bool),
    regex: RegexSearch,
    current: Option<Match>,
    /// History length and grid size when `current` was found; a change moved the grid under it.
    stamp: (usize, usize, usize),
}

fn stamp<T>(term: &Term<T>) -> (usize, usize, usize) {
    (term.history_size(), term.columns(), term.screen_lines())
}

/// The pattern for a literal `text`; the inline flag overrides the engine's smart case.
fn pattern(text: &str, match_case: bool) -> String {
    let mut out = String::from(if match_case { "(?-i)" } else { "(?i)" });
    for c in text.chars() {
        if "\\.+*?()|[]{}^$#&-~".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

impl Search {
    pub fn new(text: &str, match_case: bool) -> Option<Search> {
        if text.is_empty() {
            return None;
        }
        Some(Search {
            query: (text.to_owned(), match_case),
            regex: RegexSearch::new(&pattern(text, match_case)).ok()?,
            current: None,
            stamp: (0, 0, 0),
        })
    }

    pub fn is_for(&self, text: &str, match_case: bool) -> bool {
        self.query.0 == text && self.query.1 == match_case
    }

    /// Move to the next match after the current one (or from the view's edge when there is none),
    /// wrapping at the ends, and scroll it into view. False when nothing matches.
    pub fn step<T: EventListener>(&mut self, term: &mut Term<T>, down: bool) -> bool {
        self.drop_stale(term);
        let (origin, direction, side) = match (&self.current, down) {
            (Some(m), true) => (
                m.end().add(term, Boundary::None, 1),
                Direction::Right,
                Side::Left,
            ),
            (Some(m), false) => (
                m.start().sub(term, Boundary::None, 1),
                Direction::Left,
                Side::Right,
            ),
            (None, true) => (view_start(term), Direction::Right, Side::Left),
            (None, false) => (view_end(term), Direction::Left, Side::Right),
        };
        self.current = term.search_next(&mut self.regex, origin, direction, side, None);
        self.stamp = stamp(term);
        if let Some(m) = &self.current {
            term.scroll_to_point(*m.start());
        }
        self.current.is_some()
    }

    /// Forget the current match once the text under it is no longer the query: output scrolled
    /// the grid, history evicted lines, or a clear or resize moved them.
    fn drop_stale<T>(&mut self, term: &Term<T>) {
        let Some(m) = &self.current else {
            return;
        };
        let (a, b) = (*m.start(), *m.end());
        let in_grid = |p: Point| {
            p.line >= term.topmost_line()
                && p.line <= term.bottommost_line()
                && p.column <= term.last_column()
        };
        let same = stamp(term) == self.stamp && in_grid(a) && in_grid(b) && {
            let text = term.bounds_to_string(a, b);
            match self.query.1 {
                true => text == self.query.0,
                false => text.to_lowercase() == self.query.0.to_lowercase(),
            }
        };
        if !same {
            self.current = None;
        }
    }

    /// The matches in `term`'s viewport.
    pub fn marks<T>(&mut self, term: &Term<T>) -> Marks {
        self.drop_stale(term);
        Marks {
            all: RegexIter::new(
                view_start(term),
                view_end(term),
                Direction::Right,
                term,
                &mut self.regex,
            )
            .collect(),
            current: self.current.clone(),
        }
    }
}

fn view_start<T>(term: &Term<T>) -> Point {
    Point::new(Line(-(term.grid().display_offset() as i32)), Column(0))
}

fn view_end<T>(term: &Term<T>) -> Point {
    let top = -(term.grid().display_offset() as i32);
    Point::new(
        Line(top + term.screen_lines() as i32 - 1),
        term.last_column(),
    )
}

fn read(form: &Form) -> Query {
    Query {
        text: form.edit_text(finddlg::QUERY).to_string(),
        match_case: form.checked(finddlg::MATCH_CASE).unwrap_or(false),
        up: form.checked(finddlg::UP).unwrap_or(false),
    }
}

fn set_live(form: &mut Form, live: bool) {
    if let Some(Control::Button { enabled, .. }) =
        form.item_mut(finddlg::FIND_NEXT).map(|i| &mut i.control)
    {
        *enabled = live;
    }
}

impl Terminal {
    /// Open the Find dialog with the last query.
    pub(crate) fn open_find(&mut self) -> Reply {
        // An open Find moves to the window that asks, leaving its search behind.
        let asker = self.cur;
        if self.find.is_some() && self.find_on != asker && self.enter(self.find_on) {
            for t in &self.w.tabs.list {
                t.end_find();
            }
            self.w.snap.dirty.fill(true);
            self.enter(asker);
        }
        self.find_on = asker;
        let q = self.last_find.clone();
        self.find.get_or_insert_with(|| {
            finddlg::find_form(&FindFields {
                query: q.text,
                match_case: q.match_case,
                down: !q.up,
            })
        });
        Reply::redraw_all()
    }

    fn close_find(&mut self) -> Reply {
        self.find = None;
        for t in &self.w.tabs.list {
            t.end_find();
        }
        self.w.snap.dirty.fill(true);
        Reply::redraw_all()
    }

    fn find_next(&mut self, ctx: &mut Ctx) -> Reply {
        let Some(form) = &self.find else {
            return Reply::none();
        };
        self.last_find = read(form);
        let q = self.last_find.clone();
        let found = self
            .w
            .tabs
            .current()
            .is_some_and(|t| t.find(&q.text, q.match_case, !q.up));
        if !found && self.notfound.is_none() {
            let fields = Fields {
                text: format!("Cannot find \"{}\"", q.text),
                icon: ctx.icons.get("dialog-information", 32),
                buttons: vec!["OK".into()],
                default: 0,
                min_text_h: None,
                min_w: 0,
                max_w: repriseapp::dialog_max_w(),
            };
            self.notfound = Some(MsgBox::new("Terminal", fields));
        }
        self.w.snap.dirty.fill(true);
        Reply::redraw_all()
    }

    pub(crate) fn find_event(&mut self, ev: Event, ctx: &mut Ctx) -> Reply {
        let Some(form) = &mut self.find else {
            return Reply::none();
        };
        let clip = form_clipboard(form.focused_edit(), &ev);
        let out = match &ev {
            Event::CloseRequest => form::Event::Cancel,
            Event::Pasted(text) => form.paste(text.clone()),
            _ => match form_input(form, &ev, ctx) {
                Some(out) => out,
                None => return Reply::none(),
            },
        };
        let reply = match out {
            form::Event::Cancel | form::Event::Pressed(finddlg::CANCEL) => self.close_find(),
            form::Event::Default | form::Event::Pressed(finddlg::FIND_NEXT) => self.find_next(ctx),
            form::Event::Changed(finddlg::QUERY) => {
                set_live(form, !form.edit_text(finddlg::QUERY).is_empty());
                Reply::redraw()
            }
            form::Event::None => Reply::none(),
            _ => Reply::redraw(),
        };
        Reply {
            copy: clip.copy,
            paste: clip.paste,
            ..reply
        }
    }

    pub(crate) fn notfound_event(&mut self, ev: Event) -> Reply {
        let (Some(b), Some((text, forms))) = (&mut self.notfound, &self.forms) else {
            return Reply::none();
        };
        match b.event(&ev, text, forms) {
            oneshot::Outcome::Closed(_) => {
                self.notfound = None;
                Reply::redraw_all()
            }
            oneshot::Outcome::Redraw => Reply::redraw(),
            oneshot::Outcome::None => Reply::none(),
        }
    }

    pub(crate) fn draw_find(&self, c: &mut Canvas, ctx: &Ctx) {
        let Some(f) = &self.find else {
            return;
        };
        let forms = ctx.forms();
        let paint = Paint {
            s: 1,
            ..forms.paint()
        };
        c.fill_rect(c.bounds(), forms.colors.button_face);
        form::draw(c, &ctx.text, &paint, f, true, forms.fonts.size_px);
    }

    pub(crate) fn access_find(&self, tree: &mut Tree, ctx: &Ctx) {
        if let Some(f) = &self.find {
            oneshot::access_form(f, tree, &ctx.text, ctx.forms().fonts.size_px);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::event::VoidListener;
    use alacritty_terminal::term::Config;
    use alacritty_terminal::term::test::TermSize;
    use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};

    fn term(rows: usize, text: &str) -> Term<VoidListener> {
        let mut t = Term::new(Config::default(), &TermSize::new(20, rows), VoidListener);
        Processor::<StdSyncHandler>::new().advance(&mut t, text.as_bytes());
        t
    }

    fn at(m: &Match) -> (i32, usize) {
        (m.start().line.0, m.start().column.0)
    }

    #[test]
    fn find_next_wraps() {
        let mut t = term(4, "one foo\r\ntwo\r\nfoo three");
        let mut s = Search::new("foo", false).unwrap();
        let mut seen = Vec::new();
        for _ in 0..3 {
            assert!(s.step(&mut t, true));
            seen.push(at(s.current.as_ref().unwrap()));
        }
        assert_eq!(
            seen,
            [(0, 4), (2, 0), (0, 4)],
            "the third step wraps to the first"
        );
    }

    #[test]
    fn match_case_respected() {
        let mut t = term(3, "Foo foo");
        let mut loose = Search::new("FOO", false).unwrap();
        assert!(loose.step(&mut t, true));
        assert!(loose.step(&mut t, true));
        assert_eq!(
            at(loose.current.as_ref().unwrap()),
            (0, 4),
            "any case finds both"
        );
        let mut exact = Search::new("foo", true).unwrap();
        assert!(exact.step(&mut t, true));
        assert!(exact.step(&mut t, true));
        assert_eq!(
            at(exact.current.as_ref().unwrap()),
            (0, 4),
            "only the lower case one"
        );
        assert_eq!(exact.marks(&t).all.len(), 1);
        assert!(!Search::new("FOo", true).unwrap().step(&mut t, true));
    }

    #[test]
    fn the_query_is_literal() {
        let mut t = term(2, "a.c abc");
        let mut s = Search::new("a.c", false).unwrap();
        assert_eq!(s.marks(&t).all.len(), 1);
        assert!(s.step(&mut t, true));
        assert_eq!(at(s.current.as_ref().unwrap()), (0, 0));
    }

    #[test]
    fn up_searches_backwards() {
        let mut t = term(4, "foo 1\r\nfoo 2\r\nfoo 3");
        let mut s = Search::new("foo", false).unwrap();
        let mut seen = Vec::new();
        for _ in 0..4 {
            assert!(s.step(&mut t, false));
            seen.push(at(s.current.as_ref().unwrap()).0);
        }
        assert_eq!(
            seen,
            [2, 1, 0, 2],
            "last match first, then back, wrapping at the top"
        );
    }

    #[test]
    fn current_match_scrolled_into_view() {
        let mut text = String::from("needle\r\n");
        for i in 0..30 {
            text.push_str(&format!("line {i}\r\n"));
        }
        let mut t = term(5, &text);
        assert_eq!(t.grid().display_offset(), 0);
        let mut s = Search::new("needle", false).unwrap();
        assert!(s.step(&mut t, true));
        let line = s.current.as_ref().unwrap().start().line.0;
        let top = -(t.grid().display_offset() as i32);
        assert!(t.grid().display_offset() > 0);
        assert!(
            (top..top + 5).contains(&line),
            "line {line} is in view from {top}"
        );
    }

    #[test]
    fn matches_flagged_in_snapshot() {
        use reprisekit::controls::termgrid::{Flags, Snapshot};
        use reprisekit::theme::Color;
        let mut t = term(3, "xx foo foo");
        let mut s = Search::new("foo", false).unwrap();
        assert!(s.step(&mut t, true));
        let marks = s.marks(&t);
        let scheme = crate::schemes::scheme("reprise", reprisekit::interface::Interface::Classic);
        let black = Color::rgb(0, 0, 0);
        let mut snap = Snapshot::new(0, 0, black, black);
        crate::snapshot::fill_marked(&t, &scheme, &marks, &mut snap);
        let row = snap.row(0);
        let flags = |c: usize| {
            (
                row[c].flags.contains(Flags::MATCH),
                row[c].flags.contains(Flags::MATCH_CURRENT),
            )
        };
        assert_eq!(flags(0), (false, false));
        assert_eq!(flags(3), (true, true), "the current match");
        assert_eq!(flags(5), (true, true));
        assert_eq!(flags(7), (true, false), "another match");
        assert_eq!(flags(9), (true, false));
        assert_eq!(flags(6), (false, false), "the gap");
    }

    #[test]
    fn output_after_a_match_drops_the_current_match() {
        use reprisekit::controls::termgrid::{Flags, Snapshot};
        use reprisekit::theme::Color;
        let mut t = term(3, "foo 1\r\nxx\r\nfoo 2");
        let mut s = Search::new("foo", false).unwrap();
        assert!(s.step(&mut t, true));
        assert_eq!(at(s.current.as_ref().unwrap()), (0, 0));
        let mut p = Processor::<StdSyncHandler>::new();
        p.advance(&mut t, b"\r\nyy\r\nzz");
        let marks = s.marks(&t);
        assert!(
            marks.current.is_none(),
            "the old Point no longer holds a match"
        );
        let scheme = crate::schemes::scheme("reprise", reprisekit::interface::Interface::Classic);
        let black = Color::rgb(0, 0, 0);
        let mut snap = Snapshot::new(0, 0, black, black);
        crate::snapshot::fill_marked(&t, &scheme, &marks, &mut snap);
        assert!(
            snap.cells
                .iter()
                .all(|c| !c.flags.contains(Flags::MATCH_CURRENT))
        );
        assert!(s.step(&mut t, true));
        let m = s.current.as_ref().unwrap();
        assert_eq!(t.bounds_to_string(*m.start(), *m.end()), "foo");
    }
}
