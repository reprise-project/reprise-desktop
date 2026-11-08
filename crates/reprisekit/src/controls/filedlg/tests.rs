use super::*;
use crate::classic::widgets::{self, BarHit};
use crate::controls::hosted::{Anchor, Hosted, Spot};
use crate::controls::shellview::Field;
use crate::look::{Paint, classic_paint};
use crate::pointer::WHEEL_ROWS;
use crate::text::TextRenderer;
use crate::textedit::EditKey;
use crate::theme::{ColorScheme, Theme};
use std::rc::Rc;

mod context_menu;
mod drops;
mod extras;
mod file_names;
mod filters;
mod folder_mode;
mod keyboard;
mod marquee;
mod navigation;
mod places;
mod shortcuts;
mod sizing;
mod views;

const R: Rect = Rect::new(0, 0, CLIENT.0, CLIENT.1);

fn text() -> TextRenderer {
    TextRenderer::load(&crate::theme::FontConfig::default())
}

fn sample() -> FileDlg {
    FileDlg::new(
        Fields {
            title: "Open".into(),
            accept_label: "Open".into(),
            filters: vec!["All Files (*.*)".into()],
            mode: DlgMode::Open,
            multiple: false,
        },
        vec![Place {
            label: "My Computer".into(),
            icon: None,
            enabled: true,
            pinned: false,
        }],
    )
}

fn folder(id: FolderId, parent: Option<FolderId>, entries: Vec<shellview::Entry>) -> Folder {
    Folder {
        id,
        parent,
        crumbs: vec![Crumb {
            id,
            label: "Folder".into(),
            depth: 0,
            icon: None,
        }],
        entries,
    }
}

fn dir(name: &str) -> shellview::Entry {
    shellview::Entry {
        id: 900,
        name: name.into(),
        is_dir: true,
        kind: "File Folder".into(),
        ..Default::default()
    }
}

fn file(name: &str) -> shellview::Entry {
    shellview::Entry {
        name: name.into(),
        kind: "Text Document".into(),
        ..Default::default()
    }
}

fn multi(mode: DlgMode, multiple: bool) -> FileDlg {
    let mut d = FileDlg::new(
        Fields {
            title: "Open".into(),
            accept_label: "Open".into(),
            filters: vec!["All Files".into(), "Text".into()],
            mode,
            multiple,
        },
        vec![],
    );
    d.set_listing(folder(
        1,
        None,
        vec![dir("sub"), file("a.txt"), file("b.txt"), file("c.txt")],
    ));
    d
}

const CTRL: Mods = Mods {
    ctrl: true,
    shift: false,
};
const SHIFT: Mods = Mods {
    ctrl: false,
    shift: true,
};

fn entry_hit(i: usize) -> Option<Hit> {
    Some(Hit::Shell(shellview::Hit::Entry(i)))
}

fn with_extra() -> FileDlg {
    let mut d = multi(DlgMode::Open, false);
    let w = field_w(CLIENT.0);
    let form = form::Form::new(vec![
        form::Item::check("extra-check", FIELD_X, EXTRA_Y, "Open read-only"),
        form::Item::combo(
            "extra-combo",
            FIELD_X,
            EXTRA_Y + 20,
            w,
            &["UTF-8", "Latin-1"],
            2,
        ),
    ]);
    d.set_extra(form, EXTRA_H, &ControlMetrics::CLASSIC);
    d
}

const EXTRA_H: i32 = 70;

fn names(d: &FileDlg) -> Vec<&str> {
    d.entries().iter().map(|e| e.name.as_str()).collect()
}
