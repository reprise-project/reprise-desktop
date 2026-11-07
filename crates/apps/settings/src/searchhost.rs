//! The search field over the page list: a query shows the pages and link rows it matches, and
//! opening a found page focuses the control the query named.

use crate::SettingsApp;
use crate::registry::Page;
use repriseapp::Reply;
use repriseconf::pages::{Hit, links};
use reprisekit::controls::pagelist::Filter;

/// The list's filter for `query` and each page's hit, in `Page::all()` order; `None` for a
/// blank query.
pub(crate) fn find(query: &str) -> Option<(Filter, Vec<Option<Hit>>)> {
    if query.trim().is_empty() {
        return None;
    }
    let hits: Vec<Option<Hit>> = Page::all().map(|p| p.find(query)).collect();
    let filter = Filter {
        pages: hits.iter().map(Option::is_some).collect(),
        links: links().map(|p| p.find(query, p.label).is_some()).collect(),
    };
    Some((filter, hits))
}

/// `filter` with `absent` hidden as well; `None` while it hides nothing.
fn without(filter: Option<Filter>, absent: &[Page]) -> Option<Filter> {
    if absent.is_empty() {
        return filter;
    }
    let mut f = filter.unwrap_or_else(|| Filter {
        pages: vec![true; Page::all().count()],
        links: vec![true; links().count()],
    });
    for p in absent {
        f.pages[p.index()] = false;
    }
    Some(f)
}

impl SettingsApp {
    /// Filter the page list by `query`, leaving out the pages this machine lacks. New rows bring
    /// the open page's row into view; the same rows again leave the list where it is scrolled.
    pub(crate) fn search(&mut self, query: &str) -> Reply {
        let found = find(query);
        let filter = without(found.as_ref().map(|(f, _)| f.clone()), &self.absent);
        if let Some(list) = self.sheet.list_mut()
            && list.filter() != filter.as_ref()
        {
            list.set_filter(filter.as_ref());
            self.sheet.reveal_open();
        }
        self.search_hits = found.map(|(_, hits)| hits).unwrap_or_default();
        Reply::redraw()
    }

    /// Call after Enter in the field or a press on a found row opened a page: the control the
    /// query named takes the keyboard, unless it is missing or greyed and the list keeps it.
    pub(crate) fn search_opened(&mut self) {
        let Some(id) = self
            .search_hits
            .get(self.sheet.tabs.open)
            .copied()
            .flatten()
            .and_then(Hit::focus)
        else {
            return;
        };
        if self.sheet.focus_control(id) {
            self.blur_side_lists();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blank_query_filters_nothing() {
        assert!(find("").is_none());
        assert!(find("   ").is_none());
    }

    #[test]
    fn every_query_word_must_match_on_one_page() {
        let shown = |q: &str| -> Vec<Page> {
            let (f, _) = find(q).unwrap();
            Page::all().filter(|p| f.pages[p.index()]).collect()
        };
        assert!(shown("default browser").contains(&Page::DefaultPrograms));
        assert!(shown("mouse speed").contains(&Page::MouseTouchpad));
        assert_eq!(shown("browser wallpaper"), [], "the words sit on two pages");
        assert_eq!(
            find("do not disturb").unwrap().1[Page::Notifications.index()],
            Some(Hit::Keyword(None))
        );
        let (f, hits) = find("night").unwrap();
        assert_eq!(
            (f.pages.len(), hits.len()),
            (Page::all().count(), Page::all().count())
        );
        assert_eq!(f.links.len(), links().count());
        assert_eq!(hits[Page::NightLight.index()], Some(Hit::Label));
        let link = |q: &str, id: &str| {
            let (f, _) = find(q).unwrap();
            f.links[links().position(|p| p.id == id).unwrap()]
        };
        assert!(link("wifi", "reprise-network.desktop"));
        assert!(link("bluetooth", "reprise-bluetooth.desktop"));
        assert!(!link("wifi", "reprise-bluetooth.desktop"));
    }
}
