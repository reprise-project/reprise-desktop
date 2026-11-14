use super::*;
use crate::pages::thunderbolt::{NO_BOLT, NO_DEVICE, NO_PORT, detail};
use reprisebus::bolt::{Device, Found, Status};
use reprisekit::controls::listbox::ListBox;

fn dev(uid: &str, name: &str, status: Status, stored: bool, auto: bool) -> Device {
    Device {
        path: format!("/org/freedesktop/bolt/devices/{uid}"),
        uid: uid.into(),
        name: name.into(),
        vendor: "Acme".into(),
        status,
        stored,
        auto,
    }
}

fn found() -> Found {
    Found {
        domains: 1,
        devices: vec![
            dev("dock", "Dock", Status::Connected, false, false),
            dev("egpu", "Graphics Box", Status::Authorized, true, true),
            dev("drive", "Drive", Status::Disconnected, true, false),
        ],
    }
}

fn list(f: &Form) -> &ListBox {
    match &f.item(TB_LIST).unwrap().control {
        Control::ListBox { list, .. } => list,
        c => panic!("the devices are a list box, not {c:?}"),
    }
}

fn pick(f: &mut Form, i: usize) {
    if let Some(Control::ListBox { list, .. }) = f.item_mut(TB_LIST).map(|i| &mut i.control) {
        list.select(i, 12);
    }
}

fn status(f: &Form) -> &str {
    match &f.item("thunderbolt_status").unwrap().control {
        Control::Label { text, .. } => text,
        c => panic!("the status is a label, not {c:?}"),
    }
}

#[test]
fn the_thunderbolt_page_geometry() {
    let s = built();
    let p = Page::Thunderbolt;
    assert_eq!(rect(&s, p, "thunderbolt_note"), Rect::new(20, 44, 358, 26));
    assert_eq!(rect(&s, p, TB_LIST), Rect::new(20, 78, 358, 208));
    assert_eq!(rect(&s, p, TB_AUTHORIZE), Rect::new(222, 294, 75, 23));
    assert_eq!(rect(&s, p, TB_FORGET), Rect::new(303, 294, 75, 23));
    assert_eq!(rect(&s, p, "thunderbolt_status").y, 324);
    inside_page(&s, p);
}

#[test]
fn the_list_grows_with_the_page() {
    let a = Area { w: 500, dh: 100 };
    let f = thunderbolt(a);
    let r = f.item(TB_LIST).unwrap().rect;
    assert_eq!((r.width, r.height), (500, 308));
    assert_eq!(f.item(TB_FORGET).unwrap().rect.right(), r.right());
}

#[test]
fn the_page_greys_until_read_and_says_why_it_is_empty() {
    let mut f = thunderbolt(Area::MIN);
    assert_eq!(list(&f).items, ["Loading..."]);
    assert_eq!(list(&f).greyed, [true]);
    assert!(!enabled(&f, TB_AUTHORIZE) && !enabled(&f, TB_FORGET));

    show_thunderbolt(&mut f, None, None, false);
    assert!(list(&f).items.is_empty());
    assert_eq!(status(&f), NO_BOLT);
    assert!(!enabled(&f, TB_AUTHORIZE) && !enabled(&f, TB_FORGET));

    show_thunderbolt(&mut f, Some(&Found::default()), None, false);
    assert_eq!(status(&f), NO_PORT);
    let empty = Found {
        domains: 1,
        devices: Vec::new(),
    };
    show_thunderbolt(&mut f, Some(&empty), None, false);
    assert_eq!(status(&f), NO_DEVICE);
    show_thunderbolt(&mut f, Some(&found()), None, false);
    assert_eq!(status(&f), "");
}

#[test]
fn each_row_says_how_its_device_stands() {
    let mut f = thunderbolt(Area::MIN);
    show_thunderbolt(&mut f, Some(&found()), None, false);
    let lb = list(&f);
    assert_eq!(lb.items, ["Acme Dock", "Acme Graphics Box", "Acme Drive"]);
    assert_eq!(
        lb.detail,
        [
            "Waiting for authorization",
            "Connected, always authorized",
            "Not connected",
        ]
    );
    assert!(lb.greyed.is_empty());
    let auto_away = dev("x", "X", Status::Unknown, true, true);
    assert_eq!(detail(&auto_away), "Always authorized");
}

#[test]
fn the_buttons_follow_the_picked_device() {
    let mut f = thunderbolt(Area::MIN);
    let found = found();
    show_thunderbolt(&mut f, Some(&found), None, false);
    assert!(!enabled(&f, TB_AUTHORIZE), "nothing picked");

    pick(&mut f, 0);
    show_thunderbolt(&mut f, Some(&found), None, false);
    assert!(enabled(&f, TB_AUTHORIZE), "a waiting device");
    assert!(!enabled(&f, TB_FORGET), "not enrolled");

    pick(&mut f, 1);
    show_thunderbolt(&mut f, Some(&found), None, false);
    assert!(!enabled(&f, TB_AUTHORIZE), "boltd authorizes it already");
    assert!(enabled(&f, TB_FORGET));

    pick(&mut f, 2);
    show_thunderbolt(&mut f, Some(&found), None, false);
    assert!(enabled(&f, TB_AUTHORIZE), "enrolled to be asked each time");
    assert!(enabled(&f, TB_FORGET));
    assert_eq!(selected_device(&f, &found.devices), Some(&found.devices[2]));

    show_thunderbolt(&mut f, Some(&found), None, true);
    assert!(
        !enabled(&f, TB_AUTHORIZE) && !enabled(&f, TB_FORGET),
        "busy"
    );
}

#[test]
fn the_picked_device_stays_picked_as_the_list_changes() {
    let mut f = thunderbolt(Area::MIN);
    let before = found();
    show_thunderbolt(&mut f, Some(&before), None, false);
    pick(&mut f, 2);
    let mut after = before.clone();
    after.devices.remove(0);
    show_thunderbolt(&mut f, Some(&after), Some(&before), false);
    assert_eq!(list(&f).selected, Some(1));
    assert_eq!(selected_device(&f, &after.devices).unwrap().uid, "drive");
    assert_eq!(
        selected_device(&f, &before.devices),
        None,
        "a list of another length is not the one shown"
    );
}

#[test]
fn the_thunderbolt_page_pins_its_pixels_in_every_look() {
    let page = Rect::new(20, 40, 358, 300);
    let mut f = thunderbolt(Area::MIN);
    show_thunderbolt(&mut f, Some(&found()), None, false);
    pick(&mut f, 0);
    show_thunderbolt(&mut f, Some(&found()), None, false);
    let listed = row_pins(&f, page);
    show_thunderbolt(&mut f, None, None, false);
    let none = row_pins(&f, page);
    assert_eq!(
        [listed, none],
        [
            [0x6b75b993a083523f, 0x8644206d843fa1b5, 0x074021d4fb3f4d18],
            [0xb75eab4eda87ac25, 0x8469dcb7d427eb5f, 0x67602befd6c79841],
        ],
        "listed, no bolt; Classic, Blue, Glass"
    );
}

#[test]
fn a_device_with_another_ones_label_does_not_take_its_pick() {
    let mut f = thunderbolt(Area::MIN);
    let dock = |uid| dev(uid, "Dock", Status::Connected, false, false);
    let before = Found {
        domains: 1,
        devices: vec![dock("first"), dock("second")],
    };
    show_thunderbolt(&mut f, Some(&before), None, false);
    pick(&mut f, 0);
    let after = Found {
        devices: vec![dock("second"), dock("first")],
        ..before.clone()
    };
    show_thunderbolt(&mut f, Some(&after), Some(&before), false);
    assert_eq!(selected_device(&f, &after.devices).unwrap().uid, "first");
    let mut gone = after.clone();
    gone.devices.remove(1);
    show_thunderbolt(&mut f, Some(&gone), Some(&after), false);
    assert_eq!(list(&f).selected, None, "the pick goes with its device");
}
