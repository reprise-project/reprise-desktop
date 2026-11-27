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
            &tr!("filesheet-inert-local-sharing-and-security"),
        ),
        Item::picture("sh_local", Rect::new(31, 54, ICON_PX, ICON_PX), local),
    ];
    greyed_note(
        &mut items,
        "sh_a0",
        (SHARE_X, 53, SHARE_W),
        &[
            &tr!("filesheet-inert-to-share-this-folder-with-other"),
            &tr!("filesheet-inert-only-drag-it-to-the-public"),
        ],
    );
    greyed_note(
        &mut items,
        "sh_b0",
        (SHARE_X, 92, SHARE_W),
        &[
            &tr!("filesheet-inert-to-make-this-folder-and-its"),
            &tr!("filesheet-inert-select-the-following-check-box"),
        ],
    );
    items.push(
        Item::check(
            "sh_private",
            70,
            129,
            &tr!("filesheet-inert-make-this-folder-private"),
        )
        .enabled(false),
    );
    items.push(Item::group(
        "sh_g1",
        Rect::new(21, 161, 318, 150),
        &tr!("filesheet-inert-network-sharing-and-security"),
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
            &tr!("filesheet-inert-this-computer-shares-no-folders-on"),
            &tr!("filesheet-inert-a-file-server-such-as-samba"),
            &tr!("filesheet-inert-reprise-shares-folders"),
        ],
    );
    greyed_note(
        &mut items,
        "sh_d0",
        (SHARE_X, 237, SHARE_W),
        &[
            &tr!("filesheet-inert-if-you-understand-the-security-risks"),
            &tr!("filesheet-inert-to-the-file-server-s-shares"),
        ],
    );
    greyed_note(
        &mut items,
        "sh_more",
        (29, 287, 300),
        &[&tr!(
            "filesheet-inert-learn-more-about-sharing-and-security"
        )],
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
            &tr!("filesheet-inert-if-you-have-problems-with-this"),
            &tr!("filesheet-inert-on-an-earlier-system-select-the"),
            &tr!("filesheet-inert-matches-that-earlier-system"),
        ],
    );
    items.push(Item::group(
        "cp_g0",
        Rect::new(21, 103, 315, 73),
        &tr!("filesheet-inert-compatibility-mode"),
    ));
    items.push(
        Item::check(
            "cp_mode",
            37,
            124,
            &tr!("filesheet-inert-run-this-program-in-compatibility-mode"),
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
        &tr!("filesheet-inert-display-settings"),
    ));
    for (id, y, label) in [
        ("cp_256", 216, &tr!("filesheet-inert-run-in-256-colors")),
        (
            "cp_640",
            241,
            &tr!("filesheet-inert-run-in-640-x-480-screen"),
        ),
        (
            "cp_themes",
            265,
            &tr!("filesheet-inert-disable-visual-themes"),
        ),
    ] {
        items.push(Item::check(id, 37, y, label).enabled(false));
    }
    items.push(Item::group(
        "cp_g2",
        Rect::new(21, 311, 315, 40),
        &tr!("filesheet-inert-input-settings"),
    ));
    items.push(
        Item::check(
            "cp_text",
            37,
            327,
            &tr!("filesheet-inert-turn-off-advanced-text-services-for"),
        )
        .enabled(false),
    );
    greyed_note(
        &mut items,
        "cp_more",
        (22, 365, 300),
        &[&tr!(
            "filesheet-inert-learn-more-about-program-compatibility"
        )],
    );
    Form::new(items)
}
