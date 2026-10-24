//! The Customize page and its Change Icon box.

use super::{ICON_PX, button};
use crate::classic::widgets::{BUTTON_H, BUTTON_W};
use crate::controls::form::{Control, Form, Item};
use crate::raster::Rect;

pub const CHANGE_ICON: &str = "cu_change_icon";
pub const CUSTOM_ICON: &str = "cu_icon";
pub const CUSTOM_PREVIEW: &str = "cu_preview";
pub const PREVIEW_PX: i32 = 96;

/// Greyed: the template list and "Also apply this template to all
/// subfolders" stand for a view per kind of folder, which nothing here
/// keeps, so the list shows no template and the box no tick; Choose
/// Picture... and Restore Default, as the picture Thumbnails view would
/// put on the folder is not kept either.
pub fn customize_page() -> Form {
    let mut items = vec![
        Item::group(
            "cu_g0",
            Rect::new(21, 47, 319, 90),
            "What kind of folder do you want?",
        ),
        Item::label_at(
            "cu_template_l",
            32,
            60,
            290,
            "Use this folder type as a &template:",
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
            "Also apply this template to all &subfolders",
        )
        .enabled(false),
    );
    items.push(Item::group(
        "cu_g1",
        Rect::new(21, 151, 319, 134),
        "Folder pictures",
    ));
    items.push(Item::lines_at(
        "cu_pic_text",
        32,
        162,
        185,
        &[
            "For Thumbnails view, you can put a",
            "picture on this folder to remind you of",
            "the contents.",
        ],
    ));
    items.push(Item::label_at("cu_preview_l", 226, 162, 100, "Preview:"));
    items.push(Item::picture(
        CUSTOM_PREVIEW,
        Rect::new(225, 181, PREVIEW_PX, PREVIEW_PX),
        None,
    ));
    items.push(button(
        "cu_choose",
        Rect::new(31, 222, 98, BUTTON_H),
        "Choose &Picture...",
        false,
    ));
    items.push(button(
        "cu_restore",
        Rect::new(31, 251, 98, BUTTON_H),
        "&Restore Default",
        false,
    ));
    items.push(Item::group(
        "cu_g2",
        Rect::new(21, 299, 319, 78),
        "Folder icons",
    ));
    items.push(Item::lines_at(
        "cu_icon_text",
        32,
        312,
        300,
        &[
            "For all views except Thumbnails, you can change the standard",
            "\"folder\" icon to another icon.",
        ],
    ));
    items.push(button(
        CHANGE_ICON,
        Rect::new(31, 345, 98, BUTTON_H),
        "Change &Icon...",
        true,
    ));
    items.push(Item::picture(
        CUSTOM_ICON,
        Rect::new(251, 343, ICON_PX, ICON_PX),
        None,
    ));
    Form::new(items)
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
        Item::label_at("ci_file_l", 11, 11, 270, "&Look for icons in this file:").enabled(false),
        Item::edit("ci_file", Rect::new(11, 29, 185, 21), source, false),
        button(
            "ci_browse",
            Rect::new(203, 29, BUTTON_W, BUTTON_H),
            "&Browse...",
            false,
        ),
        Item::label_at(
            "ci_list_l",
            11,
            59,
            270,
            "&Select an icon from the list below:",
        ),
        Item::new(
            CHANGE_ICON_LIST,
            list_rect,
            Control::IconList(Box::new(icons)),
        ),
        button(
            CHANGE_ICON_RESTORE,
            Rect::new(11, 292, 105, BUTTON_H),
            "&Restore Defaults",
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
            "Cancel",
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

pub fn change_icon_restore(form: &mut Form) {
    let cm = form.controls();
    let Some(it) = form.item_mut(CHANGE_ICON_LIST) else {
        return;
    };
    let lay = crate::controls::iconlist::layout(it.placed(&cm), &cm);
    if let Control::IconList(l) = &mut it.control {
        l.pick(STANDARD_FOLDER_ICON);
        l.follow(&lay);
    }
}
