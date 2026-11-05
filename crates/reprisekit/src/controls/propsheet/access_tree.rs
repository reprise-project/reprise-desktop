use crate::access::{self, Tree, accesskit};
use crate::controls::pagelist::{self, Pick};
use crate::controls::{searchfield, tabstrip};
use crate::text::TextRenderer;
use accesskit::{NodeId, Role};

use super::{Nav, PropSheet};

impl PropSheet {
    /// The sheet's `access` part: the tabs, or the search field and the page list, then the
    /// open page as a panel labelled by its tab or item. Returns the panel, for a host that draws
    /// controls of its own on the page, and the focused node.
    pub fn access(
        &self,
        tree: &mut Tree,
        parent: NodeId,
        text: &TextRenderer,
        px: u32,
    ) -> (NodeId, Option<NodeId>) {
        let l = self.layout(text, px);
        let open = self.tabs.open;
        let field = l.search.map(|f| {
            let font = text.dialog(self.font);
            searchfield::access(&self.search, f, font, px, tree, parent, "search")
        });
        let (page, nav, role) = match (&self.nav, &l.list) {
            (Nav::List(list), Some(lay)) => {
                let nodes = pagelist::access(list, &self.tabs, lay, tree, parent, "pages");
                let nav = match list.current(open) {
                    Pick::Page(k) => nodes.pages.get(k),
                    Pick::Link(k) => nodes.links.get(k),
                };
                (
                    nodes.pages.get(open).copied().flatten(),
                    nav.copied().flatten(),
                    Role::Group,
                )
            }
            _ => {
                let tabs = tabstrip::access(&self.tabs, &l.tabs, tree, parent, "tabs");
                let tab = tabs.get(open).copied();
                (tab, tab, Role::TabPanel)
            }
        };
        let mut panel = access::node(role, l.page);
        if let Some(t) = page {
            panel.push_labelled_by(t);
        }
        // Keyed by the page, so a switch replaces the panel whole.
        let panel = tree.push(parent, ("page", open), panel);
        let focus = self.open().access(tree, panel, text, px);
        let nav = nav.filter(|_| self.tabs.focused);
        let field = field.filter(|_| self.search_focused);
        (panel, field.or(nav).or(focus))
    }
}
