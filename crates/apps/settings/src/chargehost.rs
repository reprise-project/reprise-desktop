//! The Power page's battery charge limit: read from UPower each time the page opens, and turned on
//! or off at once, without waiting for Apply.

use crate::registry::Page;
use crate::{SettingsApp, pages, registry, take};
use reprisebus::charge::{self, ChargeLimit};
use reprisekit::controls::form::Control;

/// A read or a toggle off the loop: the limit as UPower tells it afterwards, and the toggle's
/// failure, `None` inside when the password prompt was dismissed.
pub(crate) struct Charged {
    pub(crate) limit: Option<ChargeLimit>,
    pub(crate) failed: Option<Option<String>>,
}

/// UPower's limit; tests leave the system bus alone.
fn read() -> Option<ChargeLimit> {
    if cfg!(test) { None } else { charge::read() }
}

fn toggle(device: &str, on: bool) -> Charged {
    Charged {
        failed: charge::set(device, on).err(),
        limit: read(),
    }
}

impl SettingsApp {
    /// Read the limit off the loop each time the Power page opens.
    pub(crate) fn ensure_charge(&mut self) {
        if registry::open(&self.sheet) != Page::Power {
            self.charge_seen = false;
            return;
        }
        if self.charge_seen || self.charging.is_some() {
            return;
        }
        self.charge_seen = true;
        self.charging = Some(self.spawn(|| Charged {
            limit: read(),
            failed: None,
        }));
    }

    /// The check box was clicked: ask UPower, then show what it says.
    pub(crate) fn charge_toggled(&mut self) {
        let page = &self.sheet.pages[Page::Power.index()];
        let on = matches!(
            page.item(pages::CHARGE_LIMIT).map(|i| &i.control),
            Some(Control::Check { checked: true, .. })
        );
        let Some(device) = self.charge.as_ref().map(|l| l.device.clone()) else {
            return;
        };
        if self.charging.is_some() {
            self.refill_power();
            return;
        }
        self.sheet.pages[Page::Power.index()].set_enabled(pages::CHARGE_LIMIT, false);
        self.charging = Some(self.spawn(move || toggle(&device, on)));
    }

    /// A read or toggle finished. True when the page changed.
    pub(crate) fn charge_done(&mut self) -> bool {
        let Some(rx) = &self.charging else {
            return false;
        };
        let done = match take(rx) {
            Ok(None) => return false,
            Ok(Some(c)) => c,
            Err(()) => Charged {
                limit: self.charge.clone(),
                failed: None,
            },
        };
        self.charging = None;
        if let Some(Some(why)) = done.failed {
            self.fail(&format!("The charge limit could not be changed.\n\n{why}"));
        }
        self.charge = done.limit;
        self.refill_power();
        true
    }

    /// Build the Power page again for the limit now known, keeping what it holds.
    pub(crate) fn refill_power(&mut self) {
        let mut page = pages::power(self.area(), self.charge.as_ref());
        pages::write_form(&mut page, &self.want);
        let i = Page::Power.index();
        let old = &self.sheet.pages[i];
        let focus = old.focus().map(|f| old.items[f].id);
        self.sheet.set_page(i, page);
        if let Some(id) = focus.filter(|_| self.sheet.tabs.open == i && !self.sheet.tabs.focused) {
            self.sheet.pages[i].focus_id(id);
        }
    }
}
