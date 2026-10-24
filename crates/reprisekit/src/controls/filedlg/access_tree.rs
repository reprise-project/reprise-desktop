use super::{CANCEL, FileDlg, Focus};
use crate::access::{self, Tree, accesskit};
use crate::classic::widgets::{self, edit_first_visible};
use crate::controls::combo;
use crate::controls::placesbar;
use crate::controls::shellview;
use crate::controls::toolbar;
use crate::raster::Rect;
use crate::text::TextRenderer;
use accesskit::NodeId;

impl FileDlg {
    /// The dialog's `access` part, laid out in `r` like [`FileDlg::draw`].
    pub fn access(&self, tree: &mut Tree, r: Rect, text: &TextRenderer, px: u32) {
        let root = access::ROOT;
        let lay = self.layout(text, px, r, &self.drawn.0.get());
        let labels = self.labels();
        let label = |tree: &mut Tree, key: &str, rect: Rect, s: &str| {
            tree.push(root, key, access::label(rect, s, true))
        };
        let labelled = |tree: &mut Tree, id: NodeId, by: NodeId| {
            if let Some(n) = tree.get_mut(id) {
                n.push_labelled_by(by);
            }
        };
        let l = label(tree, "lookin label", lay.lookin_label, &labels.lookin);
        let lookin = combo::access(&self.lookin, &lay.lookin, text, px, tree, root, "lookin");
        labelled(tree, Tree::id_of(root, "lookin"), l);
        toolbar::access(
            &self.toolbar,
            lay.toolbar,
            &lay.toolbar_rects,
            tree,
            root,
            "toolbar",
        );
        let places = placesbar::access(
            &self.places,
            lay.places,
            &self.drawn.0.get().places,
            tree,
            root,
            "places",
        );
        let shell = shellview::access(&self.shell, &lay.shell, text, px, tree, root, "shell");
        let l = label(tree, "filename label", lay.filename_label, &labels.filename);
        let first = edit_first_visible(
            text,
            &self.filename,
            lay.filename.inset(widgets::FIELD_PAD).width,
            px,
            0,
        );
        let filename = widgets::access_edit(
            &self.filename,
            first,
            false,
            lay.filename,
            true,
            text,
            px,
            tree,
            root,
            "filename",
        );
        labelled(tree, filename, l);
        let accept = tree.push(
            root,
            "accept",
            access::button(lay.accept, &labels.accept, true, true),
        );
        let filetype = self.has_types().then(|| {
            let l = label(tree, "filetype label", lay.filetype_label, &labels.filetype);
            let filetype = combo::access(
                &self.filetype,
                &lay.filetype,
                text,
                px,
                tree,
                root,
                "filetype",
            );
            labelled(tree, Tree::id_of(root, "filetype"), l);
            filetype
        });
        let cancel = tree.push(
            root,
            "cancel",
            access::button(lay.cancel, CANCEL, true, false),
        );
        let extra = self
            .extra
            .as_ref()
            .and_then(|(f, _)| f.access(tree, root, text, px));
        tree.set_focus(match self.focus {
            Focus::FileName => filename,
            Focus::FileType => filetype.unwrap_or(filename),
            Focus::Extra => extra.or(filetype).unwrap_or(filename),
            Focus::Accept => accept,
            Focus::Cancel => cancel,
            Focus::Places => places,
            Focus::Shell => shell,
            Focus::LookIn => lookin,
        });
    }
}
