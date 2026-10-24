use crate::access::{self, Tree, accesskit};
use crate::controls::combo;
use crate::text::{DialogFont, TextRenderer};
use crate::trackbar;
use accesskit::{NodeId, Role};

use super::draw::level_text;
use super::layout::Layout;
use super::{Dir, Focus, MUTE, NOW_PLAYING, Transport, VOLUME, VolumePopup, combo_focus};

impl VolumePopup {
    /// The popup's `access` part: the Output and Input combos, a group per shown column (label, slider, Mute), and with a player a "Now Playing" group. Returns the focused node.
    pub fn access(
        &self,
        lay: &Layout,
        text: &TextRenderer,
        px: u32,
        tree: &mut Tree,
        parent: NodeId,
    ) -> NodeId {
        let text = text.dialog(DialogFont::ShellDlg);
        let mut win = access::node(Role::Window, lay.panel);
        win.set_label(VOLUME);
        let win = tree.push(parent, "volume-popup", win);
        let mut focus = win;
        for (dir, name) in [(Dir::Output, "Output"), (Dir::Input, "Input")] {
            let f = combo::access(self.combo(dir), lay.combo(dir), text, px, tree, win, name);
            if let Some(n) = tree.get_mut(Tree::id_of(win, name)) {
                n.set_label(name);
            }
            if self.focus == combo_focus(dir) {
                focus = f;
            }
        }
        for (i, (col, cl)) in self.columns.iter().zip(&lay.columns).enumerate() {
            let Some(cl) = cl else { continue };
            let live = col.volume.enabled;
            let mut g = access::node(Role::Group, cl.rect);
            // A program's name is plain text: no access key to drop.
            g.set_label(col.label.as_str());
            let g = tree.push(win, ("column", col.key.as_deref()), g);
            let mut label = access::node(Role::Label, cl.label);
            label.set_value(col.label.as_str());
            if !live {
                label.set_disabled();
            }
            let label = tree.push(g, "label", label);
            let slider = trackbar::access(&col.volume, &cl.track, tree, g, "slider");
            if let Some(n) = tree.get_mut(slider) {
                n.push_labelled_by(label);
            }
            let mut level = access::node(Role::Label, cl.level);
            level.set_value(level_text(col));
            if !live {
                level.set_disabled();
            }
            tree.push(g, "level", level);
            let mute = tree.push(
                g,
                "mute",
                access::toggle(Role::Button, cl.mute, MUTE, col.mute.into(), live),
            );
            match self.focus {
                Focus::Thumb(f) if f == i => focus = slider,
                Focus::Mute(f) if f == i => focus = mute,
                _ => {}
            }
        }
        if let (Some(np), Some(st)) = (&self.playing, &lay.strip) {
            let g = tree.push(
                win,
                "now-playing",
                access::named(Role::Group, st.rect, NOW_PLAYING),
            );
            // A track's names are plain text: an `&` in them is no access key.
            let lines = [("title", st.title, &np.title)]
                .into_iter()
                .chain(st.artist.map(|r| ("artist", r, &np.artist)));
            for (key, r, line) in lines {
                let mut n = access::node(Role::Label, r);
                n.set_value(line.as_str());
                tree.push(g, key, n);
            }
            for t in Transport::ALL {
                let (_, name) = np.glyph(t);
                let b = tree.push(
                    g,
                    ("media", t as usize),
                    access::button(st.button(t), name, np.can(t), false),
                );
                if self.focus == Focus::Media(t) {
                    focus = b;
                }
            }
        }
        focus
    }
}
