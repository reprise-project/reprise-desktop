use crate::access::{self, Tree, accesskit};
use crate::controls::{pagelist, tabstrip};
use crate::text::TextRenderer;
use accesskit::{NodeId, Role};

use super::{Nav, PropSheet};

impl PropSheet {
    /// The sheet's `access` part: the tab list (list mode), then the open page as a
    /// panel labelled by its tab or item. Returns the panel, for a host that draws
    /// controls of its own on the page, and the focused node.
    pub fn access(
        &self,
        tree: &mut Tree,
        parent: NodeId,
        text: &TextRenderer,
        px: u32,
    ) -> (NodeId, Option<NodeId>) {
        let l = self.layout(text, px);
        let (items, role) = match (&self.nav, &l.list) {
            (Nav::List(list), Some(lay)) => (
                pagelist::access(list, &self.tabs, lay, tree, parent, "pages"),
                Role::Group,
            ),
            _ => (
                tabstrip::access(&self.tabs, &l.tabs, tree, parent, "tabs"),
                Role::TabPanel,
            ),
        };
        let mut panel = access::node(role, l.page);
        if let Some(t) = items.get(self.tabs.open) {
            panel.push_labelled_by(*t);
        }
        // Keyed by the page, so a switch replaces the panel whole.
        let panel = tree.push(parent, ("page", self.tabs.open), panel);
        let focus = self.open().access(tree, panel, text, px);
        let nav = items.get(self.tabs.open).filter(|_| self.tabs.focused);
        (panel, nav.copied().or(focus))
    }
}
