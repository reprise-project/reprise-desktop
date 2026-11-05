//! The search field over the page list: a query shows the pages ([`Page::find`]) and link rows
//! ([`repriseconf::pages::Program::find`]) it matches; Enter or a press on a found page row focuses the control the
//! query named.

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

impl SettingsApp {
    /// Filter the page list by `query`. New rows bring the open page's row into view; a query
    /// showing the rows already shown leaves the list where it is scrolled.
    pub(crate) fn search(&mut self, query: &str) -> Reply {
        let found = find(query);
        let filter = found.as_ref().map(|(f, _)| f);
        if let Some(list) = self.sheet.list_mut()
            && list.filter() != filter
        {
            list.set_filter(filter);
            self.sheet.reveal_open();
        }
        self.search_hits = found.map(|(_, hits)| hits).unwrap_or_default();
        Reply::redraw()
    }

    /// Call after Enter in the field or a press on a found row opened a page: the control the
    /// query named takes the keyboard.
    pub(crate) fn search_opened(&mut self) {
        let Some(&Some(Hit::Control(id))) = self.search_hits.get(self.sheet.tabs.open) else {
            return;
        };
        self.blur_side_lists();
        self.sheet.focus_control(id);
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
            Some(Hit::Keyword)
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
