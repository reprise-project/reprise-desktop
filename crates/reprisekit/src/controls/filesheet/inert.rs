//! The Sharing and Compatibility pages: greyed notes and controls that change nothing.

use super::ICON_PX;
use crate::controls::form::{Control, Form, Item};
use crate::raster::{Canvas, Rect};

/// A greyed paragraph broken into `lines` at `(x, y)`, `w` wide: it rewraps when a line is wider.
fn greyed_note(
    items: &mut Vec<Item>,
    id: &'static str,
    (x, y, w): (i32, i32, i32),
    lines: &[&str],
) {
    items.push(Item::lines_at(id, x, y, w, lines).enabled(false));
}

const SHARE_X: i32 = 71;
const SHARE_W: i32 = 262;

/// private". Nothing on it maps to anything here: a folder is shared
/// with other users by its permissions and on the network by a file
/// server such as Samba, which Reprise does not set up, so the check
/// box shows no state.
pub fn sharing_page(local: Option<Canvas>, network: Option<Canvas>) -> Form {
    let mut items = vec![
        Item::group(
            "sh_g0",
            Rect::new(21, 43, 318, 108),
            "Local sharing and security",
        ),
        Item::picture("sh_local", Rect::new(31, 54, ICON_PX, ICON_PX), local),
    ];
    greyed_note(
        &mut items,
        "sh_a0",
        (SHARE_X, 53, SHARE_W),
        &[
            "To share this folder with other users of this computer",
            "only, drag it to the Public folder.",
        ],
    );
    greyed_note(
        &mut items,
        "sh_b0",
        (SHARE_X, 92, SHARE_W),
        &[
            "To make this folder and its subfolders private,",
            "select the following check box.",
        ],
    );
    items.push(Item::check("sh_private", 70, 129, "&Make this folder private").enabled(false));
    items.push(Item::group(
        "sh_g1",
        Rect::new(21, 161, 318, 150),
        "Network sharing and security",
    ));
    items.push(Item::picture(
        "sh_network",
        Rect::new(31, 171, ICON_PX, ICON_PX),
        network,
    ));
    greyed_note(
        &mut items,
        "sh_c0",
        (SHARE_X, 170, SHARE_W),
        &[
            "This computer shares no folders on the network.",
            "A file server such as Samba, set up outside",
            "Reprise, shares folders.",
        ],
    );
    greyed_note(
        &mut items,
        "sh_d0",
        (SHARE_X, 237, SHARE_W),
        &[
            "If you understand the security risks, add this folder",
            "to the file server's shares yourself.",
        ],
    );
    greyed_note(
        &mut items,
        "sh_more",
        (29, 287, 300),
        &["Learn more about sharing and security."],
    );
    Form::new(items)
}

/// The Compatibility page of a set with a shortcut in it, greyed: the
/// paragraph, the compatibility mode box and its list, the display and
/// input boxes. Programs here run on the system they were built for, so
/// nothing maps to them and every box shows no state.
pub fn compat_page() -> Form {
    let mut items = Vec::new();
    greyed_note(
        &mut items,
        "cp_l0",
        (22, 40, 310),
        &[
            "If you have problems with this program and it worked correctly",
            "on an earlier system, select the compatibility mode that",
            "matches that earlier system.",
        ],
    );
    items.push(Item::group(
        "cp_g0",
        Rect::new(21, 103, 315, 73),
        "Compatibility mode",
    ));
    items.push(
        Item::check(
            "cp_mode",
            37,
            124,
            "&Run this program in compatibility mode for:",
        )
        .enabled(false),
    );
    let mut mode = Item::combo("cp_list", 37, 144, 228, &[""], 1);
    if let Control::Combo(c) = &mut mode.control {
        c.enabled = false;
    }
    items.push(mode);
    items.push(Item::group(
        "cp_g1",
        Rect::new(21, 197, 315, 92),
        "Display settings",
    ));
    for (id, y, label) in [
        ("cp_256", 216, "Run in 256 colo&rs"),
        ("cp_640", 241, "Run in 640 x 480 screen resolutio&n"),
        ("cp_themes", 265, "Disable &visual themes"),
    ] {
        items.push(Item::check(id, 37, y, label).enabled(false));
    }
    items.push(Item::group(
        "cp_g2",
        Rect::new(21, 311, 315, 40),
        "Input settings",
    ));
    items.push(
        Item::check(
            "cp_text",
            37,
            327,
            "Turn off advanced text services for this program",
        )
        .enabled(false),
    );
    greyed_note(
        &mut items,
        "cp_more",
        (22, 365, 300),
        &["Learn more about program compatibility."],
    );
    Form::new(items)
}
