use super::*;
use super::{
    build::button,
    draw::draw_delete_badge,
    welcome::{
        GLASS_WELCOME_CELL, GLASS_WELCOME_MARGIN, GLASS_WELCOME_TOP, GLASS_WELCOME_W,
        WELCOME_ICON_Y,
    },
};
use crate::config::PowerConfig;
use crate::raster::Canvas;
use crate::text::{split_mnemonic, wrap};
use crate::textedit::EditKey;
use crate::theme::Color;
use reprisekit::classic::widgets::{BUTTON_H, BUTTON_W, BarHit, LINE_H};
use reprisekit::controls::combo;
use reprisekit::controls::form::Key;
use reprisekit::controls::metrics::ControlMetrics;
use reprisekit::cues::Cues;
use reprisekit::pointer::{Interaction, Lit};

mod fit;
mod frame;
mod preview;
mod prompts;
mod run;
mod welcome;

use crate::theme::FontConfig;

/// The test fonts' UI face, for tests that pin exact pixels.
/// A two-button message box over a file named `name`, Yes the default: the plain prompt the
/// frame and fit tests draw in every look.
fn yes_no(name: &str) -> Dialog {
    Dialog {
        icon: Some("user-trash-full".into()),
        text: format!("Are you sure you want to send '{name}' to the Recycle Bin?"),
        buttons: vec![
            button("&Yes", DialogAction::Cancel, true),
            button("&No", DialogAction::Cancel, false),
        ],
        ..Dialog::new(
            "Confirm File Delete",
            Style::Classic {
                template: Template::MessageBox,
                text_h: None,
                delete_badge: false,
            },
            Placement::Center,
        )
    }
}

fn tahoma() -> TextRenderer {
    reprisekit::pins::fonts();
    TextRenderer::load(&FontConfig::default())
}

fn power() -> PowerConfig {
    PowerConfig {
        standby: "systemctl suspend".into(),
        poweroff: "systemctl poweroff".into(),
        reboot: "systemctl reboot".into(),
        ..PowerConfig::default()
    }
}

/// The Log Off panel of a console session with no login link.
fn log_off() -> Dialog {
    Dialog::log_off(Leave::SwitchUser { live: false })
}

/// The Turn Off panel of a console session.
fn turn_off() -> Dialog {
    Dialog::turn_off(&power(), false)
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
        d,
        false,
        Lit::default(),
        Cues::NONE,
        true,
        true,
        None,
    )
}

fn top_panel(h: &crate::harness::Harness) -> &reprisekit::powerpanel::Panel {
    match &h.state.dialogs.views.last().expect("a dialog").dialog.style {
        Style::Welcome(w) => &w.panel,
        _ => panic!("a Welcome panel"),
    }
}

#[test]
fn the_log_off_dialog_offers_disconnect_on_the_remote_side() {
    let (mut h, _portal, _lock) = crate::console::tests::in_lock_mode();
    h.state.run_action(&Action::LogOff, 0);
    assert_eq!(top_panel(&h).bigs[0].choice, Choice::Disconnect);

    let mut h = crate::harness::Harness::new();
    h.output("eDP-1", 1920, 1080);
    h.state.run_action(&Action::LogOff, 0);
    assert_eq!(top_panel(&h).bigs[0].choice, Choice::SwitchUser);
}

#[test]
fn the_turn_off_dialog_offers_disconnect_in_stand_bys_place_with_the_focus() {
    let (mut h, _portal, _lock) = crate::console::tests::in_lock_mode();
    h.state.run_action(&Action::TurnOff, 0);
    let panel = top_panel(&h);
    let choices: Vec<Choice> = panel.bigs.iter().map(|b| b.choice).collect();
    assert_eq!(
        choices,
        [Choice::Disconnect, Choice::TurnOff, Choice::Restart]
    );
    assert_eq!(panel.focus, reprisekit::powerpanel::Part::Big(0));
    let d = &mut h.state.dialogs.views.last_mut().expect("a dialog").dialog;
    assert_eq!(d.key(Key::Enter), Outcome::Disconnect);
}
