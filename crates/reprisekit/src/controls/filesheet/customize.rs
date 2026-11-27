//! The Customize page and its Change Icon box.

use super::{ICON_PX, button};
use crate::classic::widgets::{BUTTON_H, BUTTON_W};
use crate::controls::form::{Control, Form, Item};
use crate::raster::Rect;

pub const CHANGE_ICON: &str = "cu_change_icon";
pub const CUSTOM_ICON: &str = "cu_icon";
pub const CUSTOM_PREVIEW: &str = "cu_preview";
pub const CHOOSE_PICTURE: &str = "cu_choose";
pub const RESTORE_PICTURE: &str = "cu_restore";
pub const PREVIEW_PX: i32 = 96;

/// Greyed: the template list and "Also apply this template to all
/// subfolders" stand for a view per kind of folder, which nothing here
/// keeps, so the list shows no template and the box no tick. Choose
/// Picture... and Restore Default start greyed too: one folder's sheet
/// turns them on ([`set_picture_buttons`]).
pub fn customize_page() -> Form {
    let mut items = vec![
        Item::group(
            "cu_g0",
            Rect::new(21, 47, 319, 90),
            &tr!("filesheet-customize-what-kind-of-folder-do-you"),
        ),
        Item::label_at(
            "cu_template_l",
            32,
            60,
            290,
            &tr!("filesheet-customize-use-this-folder-type-as-a"),
        )
        .enabled(false),
    ];
    let mut template = Item::combo("cu_template", 31, 80, 299, &[""], 1);
    if let Control::Combo(c) = &mut template.control {
        c.enabled = false;
    }
    items.push(template);
    items.push(
        Item::check(
            "cu_all",
            31,
            113,
            &tr!("filesheet-customize-also-apply-this-template-to-all"),
        )
        .enabled(false),
    );
    items.push(Item::group(
        "cu_g1",
        Rect::new(21, 151, 319, 134),
        &tr!("filesheet-customize-folder-pictures"),
    ));
    items.push(Item::lines_at(
        "cu_pic_text",
        32,
        162,
        185,
        &[
            &tr!("filesheet-customize-for-thumbnails-view-you-can-put"),
            &tr!("filesheet-customize-picture-on-this-folder-to-remind"),
            &tr!("filesheet-customize-the-contents"),
        ],
    ));
    items.push(Item::label_at(
        "cu_preview_l",
        226,
        162,
        100,
        &tr!("filesheet-customize-preview"),
    ));
    items.push(Item::picture(
        CUSTOM_PREVIEW,
        Rect::new(225, 181, PREVIEW_PX, PREVIEW_PX),
        None,
    ));
    items.push(button(
        CHOOSE_PICTURE,
        Rect::new(31, 222, 98, BUTTON_H),
        &tr!("filesheet-customize-choose-picture"),
        false,
    ));
    items.push(button(
        RESTORE_PICTURE,
        Rect::new(31, 251, 98, BUTTON_H),
        &tr!("filesheet-customize-restore-default"),
        false,
    ));
    items.push(Item::group(
        "cu_g2",
        Rect::new(21, 299, 319, 78),
        &tr!("filesheet-customize-folder-icons"),
    ));
    items.push(Item::lines_at(
        "cu_icon_text",
        32,
        312,
        300,
        &[
            &tr!("filesheet-customize-for-all-views-except-thumbnails-you"),
            &tr!("filesheet-customize-folder-icon"),
        ],
    ));
    items.push(button(
        CHANGE_ICON,
        Rect::new(31, 345, 98, BUTTON_H),
        &tr!("filesheet-customize-change-icon"),
        true,
    ));
    items.push(Item::picture(
        CUSTOM_ICON,
        Rect::new(251, 343, ICON_PX, ICON_PX),
        None,
    ));
    Form::new(items)
}

/// Customize's Choose Picture... takes clicks with `choose`, and Restore Default with `restore`,
/// while the folder shows a picture. A focus on a button greyed moves to Choose Picture....
pub fn set_picture_buttons(
    sheet: &mut crate::controls::propsheet::PropSheet,
    choose: bool,
    restore: bool,
) {
    for page in &mut sheet.pages {
        if page.item(CHOOSE_PICTURE).is_none() {
            continue;
        }
        let focused = page.focus().map(|i| page.items[i].id);
        page.set_enabled(CHOOSE_PICTURE, choose);
        page.set_enabled(RESTORE_PICTURE, choose && restore);
        if focused == Some(RESTORE_PICTURE) && !(choose && restore) {
            page.focus_id(CHOOSE_PICTURE);
        }
    }
}

pub const CHANGE_ICON_SIZE: (i32, i32) = (290, 326);
pub const CHANGE_ICON_LIST: &str = "ci_list";
pub const CHANGE_ICON_OK: &str = "ci_ok";
pub const CHANGE_ICON_CANCEL: &str = "ci_cancel";
pub const CHANGE_ICON_RESTORE: &str = "ci_restore";
/// The standard folder's icon, which Restore Defaults picks and which
/// writes no icon of the folder's own.
pub const STANDARD_FOLDER_ICON: &str = "folder";

pub const FOLDER_ICON_NAMES: &[&str] = &[
    "folder",
    "folder-documents",
    "folder-download",
    "folder-music",
    "folder-pictures",
    "folder-videos",
    "folder-publicshare",
    "folder-templates",
    "folder-remote",
    "user-home",
    "user-desktop",
    "user-trash",
    "computer",
    "network-workgroup",
    "drive-harddisk",
    "drive-optical",
    "drive-removable-media",
    "media-flash",
    "applications-multimedia",
    "applications-graphics",
    "applications-internet",
    "applications-office",
    "applications-games",
    "applications-development",
    "applications-system",
    "preferences-desktop",
    "help-browser",
    "emblem-favorite",
    "emblem-important",
    "text-x-generic",
    "image-x-generic",
    "audio-x-generic",
    "video-x-generic",
    "package-x-generic",
    "system-shutdown",
];

pub fn change_icon_form(
    source: &str,
    mut icons: crate::controls::iconlist::IconList,
    restore: bool,
) -> Form {
    let cm = crate::controls::metrics::ControlMetrics::CLASSIC;
    let list_rect = Rect::new(11, 77, 267, 198);
    icons.follow(&crate::controls::iconlist::layout(list_rect, &cm));
    let mut items = vec![
        Item::label_at(
            "ci_file_l",
            11,
            11,
            270,
            &tr!("filesheet-customize-look-for-icons-in-this-file"),
        )
        .enabled(false),
        Item::edit("ci_file", Rect::new(11, 29, 185, 21), source, false),
        button(
            "ci_browse",
            Rect::new(203, 29, BUTTON_W, BUTTON_H),
            &tr!("filesheet-customize-browse"),
            false,
        ),
        Item::label_at(
            "ci_list_l",
            11,
            59,
            270,
            &tr!("filesheet-customize-select-an-icon-from-the-list"),
        ),
        Item::new(
            CHANGE_ICON_LIST,
            list_rect,
            Control::IconList(Box::new(icons)),
        ),
        button(
            CHANGE_ICON_RESTORE,
            Rect::new(11, 292, 105, BUTTON_H),
            &tr!("filesheet-customize-restore-defaults"),
            true,
        ),
        Item::button(
            CHANGE_ICON_OK,
            Rect::new(122, 292, BUTTON_W, BUTTON_H),
            "OK",
            true,
            true,
        ),
        button(
            CHANGE_ICON_CANCEL,
            Rect::new(203, 292, BUTTON_W, BUTTON_H),
            &tr!("cancel"),
            true,
        ),
    ];
    if !restore {
        items.retain(|i| i.id != CHANGE_ICON_RESTORE);
    }
    let mut form = Form::new(items);
    form.focus_id(CHANGE_ICON_LIST);
    form
}

pub fn change_icon_picked(form: &Form) -> Option<String> {
    match form.item(CHANGE_ICON_LIST).map(|i| &i.control) {
        Some(Control::IconList(l)) => l.picked().map(str::to_string),
        _ => None,
    }
}

pub fn change_icon_list_mut(form: &mut Form) -> Option<&mut crate::controls::iconlist::IconList> {
    match form.item_mut(CHANGE_ICON_LIST).map(|i| &mut i.control) {
        Some(Control::IconList(l)) => Some(l),
        _ => None,
    }
}

/// Takes the icons in `gone` off Change Icon's list, keeping the pick in view.
pub fn change_icon_drop(form: &mut Form, gone: &[String]) {
    let Some(lay) = form.icon_list_layout(CHANGE_ICON_LIST) else {
        return;
    };
    if let Some(l) = change_icon_list_mut(form) {
        l.drop_names(gone, &lay);
    }
}

pub fn change_icon_restore(form: &mut Form) {
    let Some(lay) = form.icon_list_layout(CHANGE_ICON_LIST) else {
        return;
    };
    if let Some(l) = change_icon_list_mut(form) {
        l.pick(STANDARD_FOLDER_ICON);
        l.follow(&lay);
    }
}
