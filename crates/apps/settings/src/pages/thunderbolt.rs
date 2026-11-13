//! The Thunderbolt page: boltd's devices with Authorize and Forget, which act at once rather than
//! through Apply.

use super::{Area, X, button};
use reprisebus::bolt::{Device, Found, Status};
use reprisekit::classic::widgets::{BUTTON_GAP, BUTTON_W, LINE_H};
use reprisekit::controls::form::{Control, Form, Item};
use reprisekit::controls::listbox::{self, ListBox};
use reprisekit::raster::Rect;

pub const TB_LIST: &str = "thunderbolt_devices";
pub const TB_AUTHORIZE: &str = "thunderbolt_authorize";
pub const TB_FORGET: &str = "thunderbolt_forget";
const TB_NOTE: &str = "thunderbolt_note";
const TB_STATUS: &str = "thunderbolt_status";

/// The list's rows at [`Area::MIN`]; it takes all the page's extra height.
const ROWS: i32 = 12;

pub const NO_BOLT: &str = "The bolt service isn't available, so no device can be listed.";
pub const NO_PORT: &str = "This computer has no Thunderbolt or USB4 controller.";
pub const NO_DEVICE: &str = "No Thunderbolt or USB4 device has been connected.";

fn list_rect(a: Area) -> Rect {
    let y = 44 + 2 * LINE_H + 8;
    Rect::new(X, y, a.w, ROWS * listbox::ROW_H + 4 + a.dh)
}

/// The page before boltd is read: a greyed Loading... row and both buttons greyed until
/// [`show_thunderbolt`].
pub fn thunderbolt(a: Area) -> Form {
    let list = list_rect(a);
    let y = list.bottom() + 8;
    let mut page = Form::new(vec![
        Item::new(
            TB_NOTE,
            Rect::new(X, 44, a.w, 2 * LINE_H),
            Control::Lines(vec![
                "A device you authorize can reach this computer's memory.".into(),
                "Authorize lets it in from now on; Forget makes it ask again.".into(),
            ]),
        ),
        Item::new(
            TB_LIST,
            list,
            Control::ListBox {
                list: ListBox {
                    greyed: vec![true],
                    ..ListBox::new(vec!["Loading...".into()], None)
                },
                icons: Vec::new(),
            },
        )
        .named("Devices"),
        button(
            TB_AUTHORIZE,
            list.right() - 2 * BUTTON_W - BUTTON_GAP,
            y,
            "&Authorize",
        ),
        button(TB_FORGET, list.right() - BUTTON_W, y, "&Forget"),
        Item::label_at(TB_STATUS, X, y + 30, a.w, ""),
    ]);
    for id in [TB_AUTHORIZE, TB_FORGET] {
        page.set_enabled(id, false);
    }
    page
}

/// A row's dimmed second text: how the device stands now, and whether boltd authorizes it itself.
pub fn detail(d: &Device) -> String {
    let now = match d.status {
        Status::Connected => "Waiting for authorization",
        Status::Authorized => "Connected",
        Status::Disconnected => "Not connected",
        Status::Connecting => "Connecting...",
        Status::Authorizing => "Authorizing...",
        Status::AuthError => "Authorization failed",
        Status::Unknown => "",
    };
    match (d.auto, now.is_empty()) {
        (true, true) => "Always authorized".into(),
        (true, false) => format!("{now}, always authorized"),
        (false, _) => now.into(),
    }
}

/// Whether Authorize applies to `d`: anything but a device boltd already authorizes itself.
pub fn can_authorize(d: &Device) -> bool {
    !d.auto || d.waiting()
}

/// Show what boltd knows (None: it can't be reached), keeping the picked device picked, the
/// buttons greyed while `busy` carries out a change.
pub fn show_thunderbolt(page: &mut Form, found: Option<&Found>, busy: bool) {
    let devices = found.map_or(&[][..], |f| &f.devices[..]);
    let status = match found {
        None => NO_BOLT,
        Some(f) if f.domains == 0 && devices.is_empty() => NO_PORT,
        Some(_) if devices.is_empty() => NO_DEVICE,
        Some(_) => "",
    };
    let picked = picked(page, devices);
    if let Some(Control::ListBox { list, .. }) = page.item_mut(TB_LIST).map(|i| &mut i.control) {
        let items: Vec<String> = devices.iter().map(Device::label).collect();
        if list.items != items {
            let at = picked.and_then(|uid| devices.iter().position(|d| d.uid == uid));
            *list = ListBox::new(items, at);
        }
        list.detail = devices.iter().map(detail).collect();
        list.greyed = Vec::new();
    }
    let d = selected(page, devices);
    page.set_enabled(TB_AUTHORIZE, !busy && d.is_some_and(can_authorize));
    page.set_enabled(TB_FORGET, !busy && d.is_some_and(|d| d.stored));
    page.set_text(TB_STATUS, status);
}

/// The uid of the device the list has picked, while it still shows `devices`' labels.
fn picked(page: &Form, devices: &[Device]) -> Option<String> {
    let Some(Control::ListBox { list, .. }) = page.item(TB_LIST).map(|i| &i.control) else {
        return None;
    };
    let i = list.selected?;
    let label = list.items.get(i)?;
    devices
        .iter()
        .find(|d| d.label() == *label)
        .map(|d| d.uid.clone())
}

/// The device picked in the list, if the list shows `devices`.
pub fn selected<'a>(page: &Form, devices: &'a [Device]) -> Option<&'a Device> {
    let Some(Control::ListBox { list, .. }) = page.item(TB_LIST).map(|i| &i.control) else {
        return None;
    };
    let d = devices.get(list.selected?)?;
    (list.items.len() == devices.len()).then_some(d)
}
