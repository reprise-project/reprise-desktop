//! Explorer's labelled toolbar strip twice on a one-band rebar (Views hovered with Folders latched,
//! then Up held), plus the file dialog's small strip.

use reprisekit::art::viewsicon;
use reprisekit::controls::rebar;
use reprisekit::controls::toolbar::{self, Item, ToolBar};
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::TextRenderer;
use reprisekit::theme::Theme;

pub const HEIGHT: i32 = 126;

fn big(hot: usize, pressed: Option<usize>) -> ToolBar {
    let button = |id, label: Option<&str>, enabled| Item::Button {
        id,
        icon: None,
        label: label.map(str::to_string),
        enabled,
    };
    ToolBar {
        items: vec![
            Item::DropButton {
                id: toolbar::BACK,
                icon: None,
                label: Some("Back".into()),
                enabled: false,
            },
            Item::DropButton {
                id: toolbar::FORWARD,
                icon: None,
                label: Some(String::new()),
                enabled: false,
            },
            button(toolbar::UP, None, true),
            Item::Separator,
            button(toolbar::FOLDERS, Some("Folders"), true),
            Item::Separator,
            Item::DropButton {
                id: toolbar::VIEWS,
                icon: Some(viewsicon::draw24()),
                label: None,
                enabled: true,
            },
        ],
        hot: Some(hot),
        pressed,
        latched: Some(4),
        big: true,
    }
}

fn small() -> ToolBar {
    let button = |id, enabled| Item::Button {
        id,
        icon: None,
        label: None,
        enabled,
    };
    ToolBar {
        items: vec![
            Item::DropButton {
                id: toolbar::BACK,
                icon: None,
                label: None,
                enabled: true,
            },
            button(toolbar::UP, true),
            button(toolbar::NEW_FOLDER, false),
            Item::DropButton {
                id: toolbar::VIEWS,
                icon: Some(viewsicon::draw16()),
                label: None,
                enabled: true,
            },
        ],
        hot: Some(0),
        pressed: Some(1),
        latched: None,
        big: false,
    }
}

pub fn draw(c: &mut Canvas, theme: &Theme, text: &TextRenderer, r: Rect) {
    let px = theme.fonts.size_px;
    let p = theme.paint();
    let cm = theme.look.controls();
    let one_band = rebar::Rebar {
        bands: vec![rebar::Band {
            height: toolbar::height(true, &cm),
        }],
    };
    let band_h = rebar::height(&one_band, &cm.rebar);
    for (i, tb) in [big(5, None), big(4, Some(2))].iter().enumerate() {
        let rr = Rect::new(r.x, r.y + 4 + i as i32 * (band_h + 4), 420, band_h);
        rebar::draw(c, &p, &one_band, rr);
        let band = rebar::layout(&one_band, rr, &cm.rebar)[0];
        let rects = toolbar::layout(tb, text, px, band, &cm);
        toolbar::draw(c, text, &p, tb, band, &rects, px);
    }
    let tb = small();
    let strip = Rect::new(r.x + 10, r.y + 12 + 2 * (band_h + 4), 200, toolbar::HEIGHT);
    let rects = toolbar::layout(&tb, text, px, strip, &cm);
    toolbar::draw(c, text, &p, &tb, strip, &rects, px);
}
