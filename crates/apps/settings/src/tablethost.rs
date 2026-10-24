use crate::registry::Page;
use crate::{SettingsApp, registry, tabletpage};
use repriseapp::Reply;
use reprisekit::controls::form;

impl SettingsApp {
    pub(crate) fn ensure_pads(&mut self) -> bool {
        if registry::open(&self.sheet) != Page::Tablet {
            self.pads_fresh = false;
            return false;
        }
        if self.pads_fresh {
            return false;
        }
        self.pads_fresh = true;
        self.pads = reprisebus::tray::tablet_pads().unwrap_or_else(|e| {
            log::warn!("tablet pads: {e}");
            Vec::new()
        });
        if self.pad_picked >= self.pads.len() {
            self.pad_picked = 0;
        }
        self.refill_tablet();
        true
    }

    fn refill_tablet(&mut self) {
        let page = tabletpage::page(
            self.area(),
            &self.pads,
            self.pad_picked,
            &self.want.tablet_pads,
        );
        self.sheet.set_page(Page::Tablet.index(), page);
    }

    pub(crate) fn tablet_changed(&mut self, id: &str) -> Reply {
        let page = &self.sheet.pages[Page::Tablet.index()];
        let row = page.combo_selected(id).unwrap_or(0);
        if id == tabletpage::PAD {
            self.pad_picked = row.min(self.pads.len().saturating_sub(1));
            self.refill_tablet();
            self.sheet.pages[Page::Tablet.index()].focus_id(tabletpage::PAD);
            return Reply::redraw();
        }
        let Some(pad) = self.pads.get(self.pad_picked) else {
            return Reply::none();
        };
        let list = &mut self.want.tablet_pads;
        let step = match id {
            tabletpage::RING_STEP => Some(tabletpage::Step::Ring),
            tabletpage::STRIP_STEP => Some(tabletpage::Step::Strip),
            _ => None,
        };
        if let Some(step) = step {
            if let Some(form::Control::UpDown(u)) = page.item(id).map(|i| &i.control) {
                tabletpage::set_step(list, &pad.name, step, u.value());
                self.sync_apply();
            }
            return Reply::redraw();
        }
        let Some(i) = tabletpage::ROW_IDS.iter().position(|r| *r == id) else {
            return Reply::none();
        };
        let Some(&tabletpage::Row::Bind(slot)) = tabletpage::rows_of(pad).get(i) else {
            return Reply::none();
        };
        let had = tabletpage::entry_for(list, &pad.name).map_or("", |e| list[e].get(slot));
        let value = tabletpage::value_at(row, had);
        tabletpage::bind(list, &pad.name, slot, &value);
        self.sync_apply();
        Reply::redraw()
    }
}
