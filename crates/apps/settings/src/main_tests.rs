use super::*;
use crate::permissions::{Held, Kind, by_id, by_token, permission_answers};
use repriseapp::{BTN_LEFT, BTN_RIGHT, Extended, KeyInput, Mods, PointerInput};
use repriseconf::pages::Hit;
use reprisekit::classic::widgets::BarHit;
use reprisekit::controls::{form, listview, pagelist, propsheet};
use reprisekit::frame::{Chrome, FrameGeometry};
use reprisekit::interface::Interface;
use reprisekit::textedit::keysym;
use reprisekit::theme::FontConfig;
use std::sync::atomic::Ordering;

#[path = "main_tests/arguments.rs"]
mod arguments;
#[path = "main_tests/background.rs"]
mod background;
#[path = "main_tests/config_file.rs"]
mod config_file;
#[path = "main_tests/display.rs"]
mod display;
#[path = "main_tests/keyboard.rs"]
mod keyboard;
#[path = "main_tests/page_controls.rs"]
mod page_controls;
#[path = "main_tests/privacy.rs"]
mod privacy;
#[path = "main_tests/programs.rs"]
mod programs;
#[path = "main_tests/proxy_page.rs"]
mod proxy_page;
#[path = "main_tests/search.rs"]
mod search;
#[path = "main_tests/sizes.rs"]
mod sizes;

fn tmp(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!(
        "reprise-settings-main-{tag}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).expect("temp dir");
    p
}

fn app(tag: &str) -> SettingsApp {
    SettingsApp::open(tmp(tag).join("config.toml"), "us".into())
}

pub(super) fn test_ctx() -> Ctx {
    Ctx {
        theme: Theme::default(),
        text: TextRenderer::load(&FontConfig::default()).into(),
        icons: reprisekit::icontheme::IconCache::new(IconTheme::load("test", &[])),
        caret_on: true,
        size: (10, 10),
        maximized: false,
        origin: (0, 0),
        wake: repriseapp::Waker::noop(),
        heads: repriseapp::heads::HeadRequests::default(),
        want_tick: Default::default(),
        output_size: None,
        at_pick: false,
        text_size: 100,
    }
}

const MAIN: WinId = repriseapp::MAIN;

fn page_request(name: &str) -> instance::Request {
    instance::Request {
        action: Some((PAGE_ACTION.into(), vec![name.into()])),
        handoff: Default::default(),
    }
}

/// A left press at `time` on side list row `row`, found by walking down the list's body, in a
/// window that shows the whole sheet.
fn press_side_row(a: &mut SettingsApp, ctx: &mut Ctx, row: usize, time: u32) {
    ctx.size = (1920, 1080);
    let (list, rect, _) = a.side_list().expect("a page with a side list");
    let lay = listview::layout(list, rect, &ctx.forms().controls());
    let x = lay.body.x + 8;
    let y = (lay.body.y..lay.body.bottom())
        .find(|&y| list.hit(&lay, x, y) == listview::Hit::Row(row))
        .expect("the row is on screen");
    let press = PointerInput::Press {
        x,
        y,
        button: BTN_LEFT,
        time,
        mods: Mods::default(),
    };
    a.event(MAIN, Event::Pointer(press), ctx);
}

/// A key press of `sym` with `mods`, typing `text` when it types, as the window reads it.
fn key_input(sym: u32, mods: Mods, text: Option<&str>) -> KeyInput {
    KeyInput {
        sym,
        utf8: text.map(str::to_string),
        mods,
        edit: reprisekit::textedit::key_for(sym, mods.ctrl, mods.shift, text),
    }
}

fn ctrl(ch: u8) -> KeyInput {
    let mods = Mods {
        ctrl: true,
        ..Mods::default()
    };
    key_input(u32::from(ch), mods, None)
}

fn typed(a: &mut SettingsApp, ctx: &mut Ctx, text: &str) {
    a.event(
        MAIN,
        Event::Key(key_input(0, Mods::default(), Some(text))),
        ctx,
    );
}

fn press_key(a: &mut SettingsApp, ctx: &Ctx, sym: u32, mods: Mods) {
    a.key(key_input(sym, mods, None), &ctx.forms().controls());
}

fn plain_key(a: &mut SettingsApp, ctx: &Ctx, sym: u32) {
    press_key(a, ctx, sym, Mods::default());
}

fn tab(a: &mut SettingsApp, ctx: &Ctx, back: bool) {
    let mods = Mods {
        shift: back,
        ..Mods::default()
    };
    press_key(a, ctx, keysym::TAB, mods);
}

/// Which of the search field, a side list, the page list and a page control hold the keyboard.
fn holders(a: &SettingsApp) -> [bool; 4] {
    [
        a.sheet.search_focused,
        a.list_focus || a.shortcuts_focus || a.types_focus || a.permissions_focus,
        a.sheet.tabs.focused,
        a.sheet.open().focus().is_some(),
    ]
}
const SIDE_LIST: [bool; 4] = [false, true, false, false];
const PAGE: [bool; 4] = [false, false, false, true];

fn remote_read(a: &mut SettingsApp, remote: remotehost::Remote, failed: Option<String>) {
    let (tx, rx) = std::sync::mpsc::channel();
    tx.send(remotehost::Reading { remote, failed }).unwrap();
    a.remote_job = Some(rx);
    assert!(a.remote_done());
}

fn focused_id(a: &SettingsApp) -> Option<&'static str> {
    let page = a.sheet.open();
    page.focus().map(|f| page.items[f].id)
}

/// Ctrl+F, then `query` typed into the search field.
fn find(a: &mut SettingsApp, ctx: &mut Ctx, query: &str) {
    a.event(MAIN, Event::Key(ctrl(b'f')), ctx);
    assert!(a.sheet.search_focused);
    typed(a, ctx, query);
}
