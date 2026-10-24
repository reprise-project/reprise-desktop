use super::*;
use super::{
    build::button,
    draw::draw_delete_badge,
    layout::{CHOOSER_H, CHOOSER_W},
    sheet::props_changes,
    welcome::{
        GLASS_WELCOME_CELL, GLASS_WELCOME_MARGIN, GLASS_WELCOME_TOP, GLASS_WELCOME_W, PANEL_BAND,
        PANEL_DISABLED, PANEL_H, PANEL_LINE_RUNS, PANEL_W, SEPARATOR_RUNS, WELCOME_BORDER,
        WELCOME_ICON_Y, WELCOME_W, panel_gradient, welcome_title_text,
    },
};
use crate::config::PowerConfig;
use crate::raster::Canvas;
use crate::text::{split_mnemonic, wrap};
use crate::textedit::EditKey;
use crate::theme::{Color, TextUnits};
use reprisefs::bincap::BinLack;
use reprisekit::classic::widgets::{BUTTON_H, BUTTON_W, BarHit, LINE_H};
use reprisekit::controls::combo;
use reprisekit::controls::filesheet::Subject;
use reprisekit::controls::form::Key;
use reprisekit::controls::grouplist;
use reprisekit::controls::metrics::ControlMetrics;
use reprisekit::controls::openwith::{Check, Program};
use reprisekit::cues::Cues;
use reprisekit::pointer::{Interaction, Lit};

mod chooser;
mod fit;
mod frame;
mod preview;
mod prompts;
mod run;
mod sheet;
mod welcome;

use crate::theme::FontConfig;

/// A renderer for tests that pin exact pixels, which hold only for the
/// default UI face; None (the test skips) without it.
fn tahoma() -> Option<TextRenderer> {
    let text = TextRenderer::load(&FontConfig::default());
    reprisekit::text::test_tahoma().then_some(text)
}

fn power() -> PowerConfig {
    PowerConfig {
        standby: "systemctl suspend".into(),
        poweroff: "systemctl poweroff".into(),
        reboot: "systemctl reboot".into(),
        ..PowerConfig::default()
    }
}

fn tree_of(
    d: &Dialog,
) -> (
    reprisekit::access::Tree,
    reprisekit::access::accesskit::NodeId,
) {
    let theme = Theme::default();
    let text = TextRenderer::load(&FontConfig::default());
    let layout = DialogLayout::compute(&theme, &text, d);
    let mut t = reprisekit::access::Tree::window(&d.title, layout.size);
    let focus = access(d, &layout, &text, theme.fonts.size_px, &mut t);
    (t, focus)
}

fn history() -> Vec<String> {
    ["notepad", "mspaint", "calc", "cmd", "regedit"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

fn file_info() -> reprisefs::props::FileInfo {
    reprisefs::props::FileInfo {
        name: "New Text Document".into(),
        hidden_ext: ".txt".into(),
        path: "/home/u/Desktop/New Text Document.txt".into(),
        is_dir: false,
        type_name: "Plain text document".into(),
        opens_with: Some("Micro".into()),
        opens_with_icon: Some("text-editor".into()),
        location: "/home/u/Desktop".into(),
        size: 0,
        on_disk: 0,
        contains: None,
        created: Some(1_788_998_400),
        modified: 1_788_998_400,
        accessed: 1_788_998_400,
        read_only: false,
        executable: false,
        can_set_executable: true,
        hidden: false,
        listed: false,
        is_shortcut: false,
        robots: None,
        summary: None,
        own_icon: None,
        launcher: None,
        description: None,
    }
}

fn sheet_form(d: &Dialog) -> &reprisekit::controls::form::Form {
    d.form().and_then(|f| f.open_page()).expect("a sheet page")
}

fn chooser() -> Dialog {
    let app = |i: usize, n: &str| Program {
        id: format!("app{i}.desktop"),
        name: n.into(),
        icon: Some("text-x-generic".into()),
    };
    let others: Vec<_> = (0..12)
        .map(|i| app(10 + i, &format!("Program {i:02}")))
        .collect();
    Dialog::open_with(
        std::path::Path::new("/home/u/Desktop/New Text Document.txt"),
        Some("text/plain".into()),
        vec![app(1, "Micro"), app(2, "Vim")],
        others,
        None,
        false,
    )
}

fn attr_confirm() -> Dialog {
    let theme = Theme::default();
    let text = TextRenderer::load(&FontConfig::default());
    Dialog::confirm_attributes(
        AttrConfirm {
            write: AttrWrite::One {
                path: "/home/u/Desktop/New Folder".into(),
                name: None,
                hidden: None,
                folder_icon: None,
            },
            attrs: reprisefs::props::AttrMoves {
                read_only: Some(true),
                ..Default::default()
            },
            close: false,
        },
        &text,
        theme.fonts.size_px,
        TextUnits::ONE,
    )
}

fn a_set() -> reprisefs::props::SetInfo {
    let mut folder = file_info();
    folder.path = "/home/u/Desktop/Reports".into();
    folder.is_dir = true;
    folder.type_name = "File Folder".into();
    let mut other = file_info();
    other.path = "/home/u/Desktop/b.txt".into();
    other.read_only = true;
    reprisefs::props::glance_set(&[file_info(), other, folder])
}

fn blue() -> Theme {
    reprisekit::interfaces::theme_for(reprisekit::interface::Interface::Blue)
}

fn render(theme: &Theme, text: &TextRenderer, l: &DialogLayout, d: &Dialog) -> Canvas {
    draw(
        theme,
        text,
        l,
        None,
        None,
        &[],
        &[],
        None,
        d,
        false,
        Lit::default(),
        Cues::NONE,
        true,
        None,
    )
}
