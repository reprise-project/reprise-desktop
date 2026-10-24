use reprisekit::controls::notifysheet::access_sheet;
use reprisekit::controls::tooltip::{NotifySays, access_parts};
use std::time::Instant;

use crate::compositor::State;
use crate::raster::Rect;
use crate::tray::notify::{DEFAULT_ACTION, Urgency};

use super::button_labels;
use super::layout::{Drawn, lay_out_open};
use super::paint::relative;
use super::stack::Item;

/// The balloons' tree, kept while no balloon is up: an alert added to a tree a reader already
/// holds is announced, one in a new tree is not.
pub fn access(state: &State) -> reprisekit::access::Tree {
    use reprisekit::access::{ROOT, accesskit::Role};
    const NAME: &str = "Notifications";
    let v = &state.notify;
    let Some(output) = state.first_output().filter(|_| !v.stack.is_empty()) else {
        return crate::access::root(Role::Window, NAME, Rect::default());
    };
    let theme = &state.config.theme;
    let edge = state.bar_placement(&output).edge;
    let shape = state.config.glass.notification.shape();
    let now = Instant::now();
    let laid: Vec<(Drawn, Rect)> = v
        .stack
        .iter()
        .map(|b| {
            let when = relative(b.shown_at.elapsed());
            let (d, _) = lay_out_open(
                theme,
                &state.text,
                &b.item,
                b.reply.is_some(),
                edge,
                shape,
                &when,
                b.motion.open.value(now),
            );
            let ((ox, oy), (w, h)) = (d.origin(), d.size());
            (d, Rect::new(b.at.0 - ox, b.at.1 - oy, w, h))
        })
        .collect();
    let chip = v
        .chip
        .as_ref()
        .map(|c| (c.more, Rect::new(c.at.0, c.at.1, c.size.0, c.size.1)));
    let rect = laid
        .iter()
        .map(|(_, r)| *r)
        .chain(chip.map(|c| c.1))
        .reduce(|a, r| a.union(&r))
        .unwrap_or_default();
    let mut tree = crate::access::root(Role::Window, NAME, rect);
    let px = theme.fonts.size_px;
    let mut focus = None;
    for (b, (drawn, r)) in v.stack.iter().zip(&laid) {
        let off = (r.x - rect.x, r.y - rect.y);
        let labels;
        let lines;
        let says = match &b.item {
            Item::Note(n) => {
                labels = button_labels(n);
                NotifySays {
                    app: &n.app,
                    title: &n.summary,
                    body: &n.body,
                    actions: &labels,
                    opens: n.has_action(DEFAULT_ACTION),
                    urgent: n.urgency == Urgency::Critical,
                }
            }
            Item::Burst(burst) => {
                lines = (
                    burst.title(),
                    Vec::from_iter(burst.lines.iter().map(String::as_str)).join("\n"),
                );
                NotifySays {
                    app: "",
                    title: &lines.0,
                    body: &lines.1,
                    actions: &[],
                    opens: true,
                    urgent: false,
                }
            }
        };
        let reply = b.reply.as_ref();
        let node = match drawn {
            Drawn::Balloon(lay) => access_parts(
                b.serial,
                &lay.parts().offset(off.0, off.1),
                &says,
                reply,
                &state.text,
                px,
                &mut tree,
                ROOT,
            ),
            Drawn::Sheet(lay) => access_sheet(
                b.serial,
                lay,
                off,
                &says,
                reply,
                &state.text,
                px,
                &mut tree,
                ROOT,
            ),
        };
        if reply.is_some() || focus.is_none() {
            focus = Some(node);
        }
    }
    if let Some((more, r)) = chip {
        let at = Rect::new(r.x - rect.x, r.y - rect.y, r.width, r.height);
        let name = format!("{more} more notifications");
        tree.push(
            ROOT,
            "chip",
            reprisekit::access::button(at, &name, true, false),
        );
    }
    tree.set_focus(focus.unwrap_or(ROOT));
    tree
}
