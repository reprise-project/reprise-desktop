//! The window's tabs in strip order, kept in step with the strip's labels and open tab.

use reprisekit::controls::tabstrip::{DocLayout, TabStrip};

use crate::tab::Tab;

pub struct Tabs<T = Tab> {
    pub list: Vec<T>,
    pub strip: TabStrip,
}

impl<T> Tabs<T> {
    pub fn new() -> Tabs<T> {
        Tabs {
            list: Vec::new(),
            strip: TabStrip::document(Vec::new(), 0),
        }
    }

    /// Add `t` at the end and open it.
    pub fn open(&mut self, t: T, label: String) {
        self.list.push(t);
        self.strip.labels.push(label);
        self.select(self.list.len() - 1);
    }

    /// Close tab `i`; its right neighbour opens, else its left. True when no tab is left.
    pub fn close(&mut self, i: usize) -> bool {
        if i >= self.list.len() {
            return self.list.is_empty();
        }
        self.list.remove(i);
        self.strip.labels.remove(i);
        let s = &mut self.strip;
        if i < s.open || s.open >= self.list.len() {
            s.open = s.open.saturating_sub(1);
        }
        (s.hot, s.hot_close, s.dragging) = (None, None, None);
        self.list.is_empty()
    }

    pub fn select(&mut self, i: usize) {
        if i < self.list.len() {
            self.strip.open = i;
            self.strip.hot = None;
        }
    }

    #[cfg(test)]
    pub fn move_to(&mut self, from: usize, to: usize) {
        let n = self.list.len();
        if from < n && to < n && from != to {
            let t = self.list.remove(from);
            self.list.insert(to, t);
            self.strip.move_tab(from, to);
        }
    }

    pub fn current(&self) -> Option<&T> {
        self.list.get(self.strip.open)
    }

    /// The held tab follows the pointer to `x` ([`TabStrip::drag_to`]); true when the order changed.
    pub fn drag_to(&mut self, l: &DocLayout, x: i32) -> bool {
        let Some(from) = self.strip.dragging.map(|d| d.tab) else {
            return false;
        };
        if from >= self.list.len() {
            self.strip.dragging = None;
            return false;
        }
        if !self.strip.drag_to(l, x) {
            return false;
        }
        let to = self.strip.dragging.map_or(from, |d| d.tab);
        let t = self.list.remove(from);
        self.list.insert(to, t);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reprisekit::controls::tabstrip::{self, Drag};
    use reprisekit::raster::Rect;
    use reprisekit::text::TextRenderer;
    use reprisekit::theme::{FontConfig, TextUnits};

    fn three() -> Tabs<&'static str> {
        let mut t = Tabs::new();
        for n in ["a", "b", "c"] {
            t.open(n, n.into());
        }
        t
    }

    fn order(t: &Tabs<&'static str>) -> Vec<&'static str> {
        assert_eq!(
            t.strip.labels,
            t.list.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            "the strip's labels follow the list"
        );
        t.list.clone()
    }

    #[test]
    fn open_adds_at_the_end_and_opens() {
        let t = three();
        assert_eq!(order(&t), ["a", "b", "c"]);
        assert_eq!(t.strip.open, 2);
        assert_eq!(t.current(), Some(&"c"));
    }

    #[test]
    fn close_last_tab_quits() {
        let mut t = Tabs::new();
        t.open("a", "a".into());
        assert!(t.close(0), "no tab is left");
        assert!(t.list.is_empty() && t.strip.labels.is_empty());
        assert!(!three().close(1), "two tabs are left");
    }

    #[test]
    fn close_open_tab_selects_right_then_left() {
        let mut t = three();
        t.select(1);
        t.close(1);
        assert_eq!(order(&t), ["a", "c"]);
        assert_eq!(t.current(), Some(&"c"), "the right neighbour opens");
        t.close(1);
        assert_eq!(
            t.current(),
            Some(&"a"),
            "the last tab's left neighbour opens"
        );
    }

    #[test]
    fn closing_another_tab_keeps_the_open_one() {
        let mut t = three();
        t.select(2);
        t.close(0);
        assert_eq!(t.current(), Some(&"c"));
        t.select(0);
        t.close(1);
        assert_eq!(t.current(), Some(&"b"));
    }

    #[test]
    fn move_to_carries_the_open_tab() {
        let mut t = three();
        t.select(0);
        t.move_to(0, 2);
        assert_eq!(order(&t), ["b", "c", "a"]);
        assert_eq!(t.current(), Some(&"a"));
    }

    #[test]
    fn a_drag_reorders_the_list_with_the_strip() {
        let mut t = three();
        t.select(0);
        let text = TextRenderer::load(&FontConfig::default());
        let area = Rect::new(0, 0, 600, tabstrip::doc_height(TextUnits::ONE));
        let l = tabstrip::doc_layout(&text, 11, &t.strip, area, TextUnits::ONE);
        let r = l.tabs[0];
        t.strip.dragging = Some(Drag {
            tab: 0,
            grab: 5,
            x: r.x + 5,
        });
        assert!(
            t.drag_to(&l, l.tabs[1].x + 15),
            "its middle passes the next tab's"
        );
        assert_eq!(order(&t), ["b", "a", "c"]);
        assert_eq!(t.current(), Some(&"a"));
    }
}
