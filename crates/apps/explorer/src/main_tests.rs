//! Navigation command tests over a temporary folder; kept out of `main.rs` to keep it under 800 lines.

use super::*;
use crate::commands::Command;
use reprisekit::controls::treeview::{self, Node};
use reprisekit::theme::FontConfig;

#[path = "main_tests/actions.rs"]
mod actions;
#[path = "main_tests/address.rs"]
mod address;
#[path = "main_tests/computer.rs"]
mod computer;
#[path = "main_tests/items.rs"]
mod items;
#[path = "main_tests/listing.rs"]
mod listing;
#[path = "main_tests/opening.rs"]
mod opening;
#[path = "main_tests/picker.rs"]
mod picker;
#[path = "main_tests/picker_folders.rs"]
mod picker_folders;
#[path = "main_tests/pointer.rs"]
mod pointer;
#[path = "main_tests/renaming.rs"]
mod renaming;
#[path = "main_tests/sizes.rs"]
mod sizes;
#[path = "main_tests/startup.rs"]
mod startup;

/// The process's scratch home for tests: state, cache, data (the Recycle Bin) and config.
fn scratch() -> PathBuf {
    std::env::temp_dir().join(format!("reprise-explorer-tests-{}", std::process::id()))
}

/// Point the Recycle Bin at the scratch home's trash, with no drives and default settings, so a
/// test that deletes or lists the bin never reaches the developer's own. Run once per process by
/// [`test_env`], and again by a test that pointed it elsewhere, as it ends.
pub(crate) fn scratch_bin() {
    reprisefs::trash::set_mount_table(Some("/dev/null".into()));
    reprisefs::bincap::set_config_path(Some(scratch().join("config.toml")));
}

/// Saves land in a scratch state dir, thumbnails in a scratch cache dir and the Recycle Bin in
/// [`scratch_bin`], never the developer's own; set once per process, before a test changes them.
pub(crate) fn test_env() {
    static SCRATCH: std::sync::Once = std::sync::Once::new();
    SCRATCH.call_once(|| {
        let state = scratch();
        // SAFETY: tests run on parallel threads, but std serialises its own
        // env access and the other `test_env` callers wait on the `Once`.
        unsafe {
            std::env::set_var("XDG_STATE_HOME", &state);
            std::env::set_var("XDG_CACHE_HOME", state.join("cache"));
            std::env::set_var("XDG_DATA_HOME", state.join("data"));
        }
        scratch_bin();
        // A script opened in a test runs under this terminal, which records its arguments; started
        // with none, it marks the folder it opened in.
        use std::os::unix::fs::PermissionsExt;
        std::fs::create_dir_all(&state).expect("mkdir");
        let term = state.join("term");
        let argv = state.join("term-argv");
        let script = format!(
            "#!/bin/sh\n[ $# -eq 0 ] && exec touch opened-here\n\
             printf '%s\\n' \"$@\" >'{}'\nshift\nexec \"$@\"\n",
            argv.display()
        );
        std::fs::write(&term, script).expect("write");
        std::fs::set_permissions(&term, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        let config = state.join("config.toml");
        std::fs::write(&config, format!("terminal = \"{}\"\n", term.display())).expect("write");
        repriseconf::set_config_path(Some(config));
    });
}

/// A `Ctx` for the navigation commands, in [`test_env`]; its icon theme is a name nothing carries,
/// so every lookup answers `None`.
pub(crate) fn test_ctx() -> Ctx {
    test_env();
    Ctx {
        theme: Theme::default(),
        text: TextRenderer::load(&FontConfig::default()).into(),
        icons: reprisekit::icontheme::IconCache::new(reprisekit::icontheme::IconTheme::load(
            "test",
            &[],
        )),
        caret_on: true,
        size: (792, 573),
        maximized: false,
        origin: (0, 0),
        wake: repriseapp::Waker::noop(),
        heads: Default::default(),
        want_tick: Default::default(),
        output_size: None,
        at_pick: false,
        text_size: 100,
    }
}

/// An Explorer on default settings and no `.desktop` index.
pub(crate) fn test_explorer(start: Place, ctx: &mut Ctx) -> Explorer {
    Explorer::with_settings(start, Settings::default(), Vec::new().into(), None, ctx)
}

fn press(ex: &mut Explorer, sym: u32, ctx: &mut Ctx) -> Reply {
    ex.key(
        repriseapp::KeyInput {
            sym,
            utf8: None,
            mods: repriseapp::Mods::default(),
            edit: reprisekit::textedit::key_for(sym, false, false, None),
        },
        ctx,
    )
}

/// A bare `Item` for the pure sorting and Details tests.
fn item(name: &str, is_dir: bool, size: u64, kind: &str, modified: Option<i64>) -> Item {
    Item {
        name: name.to_string(),
        path: PathBuf::from("/tmp").join(name),
        is_dir,
        is_link: false,
        exec: false,
        program: None,
        hidden: false,
        size,
        modified,
        mime: None,
        kind: kind.to_string(),
        drive: None,
        mount_root: false,
        trashed: None,
        shortcut: None,
    }
}

/// A key as the seat delivers it, with the edit key the toolkit derives
/// from it.
fn key_in(sym: u32) -> repriseapp::KeyInput {
    repriseapp::KeyInput {
        sym,
        utf8: None,
        mods: repriseapp::Mods::default(),
        edit: reprisekit::textedit::key_for(sym, false, false, None),
    }
}

/// An Explorer on a temp folder holding `a.txt` and the folder `sub`,
/// with `a.txt` selected. `tempdir` names start with a dot, so hidden
/// entries show and the picker's tree can walk down to it.
fn browse_fixture(ctx: &mut Ctx) -> (tempfile::TempDir, Explorer) {
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::write(dir.path().join("a.txt"), "x").expect("write");
    std::fs::create_dir(dir.path().join("sub")).expect("mkdir");
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), ctx);
    ex.settings.show_hidden = true;
    let a = ex
        .items
        .iter()
        .position(|it| it.name == "a.txt")
        .expect("a.txt");
    ex.sv.selection.set_only(Some(a));
    (dir, ex)
}

/// The picker's layout and a copy of its tree, for a test to aim at.
fn browse_state(
    ex: &Explorer,
    ctx: &Ctx,
) -> (reprisekit::controls::browsefolder::Layout, TreeView) {
    let Some(Dialog::Browse(b)) = ex.dialog.as_ref() else {
        panic!("the Browse dialog is what is up");
    };
    let lay =
        b.bf.layout(&ex.dialog_text, ex.dialog_px, &ctx.theme.controls());
    (lay, b.bf.tree.clone())
}

fn press_at(x: i32, y: i32, time: u32) -> Event {
    Event::Pointer(repriseapp::PointerInput::Press {
        x,
        y,
        button: BTN_LEFT,
        time,
        mods: repriseapp::Mods::default(),
    })
}

fn release_at(x: i32, y: i32) -> Event {
    Event::Pointer(repriseapp::PointerInput::Release {
        x,
        y,
        button: BTN_LEFT,
        mods: repriseapp::Mods::default(),
    })
}

/// A left click on the picker at (`x`, `y`): the press, then the release
/// a button acts on.
fn click_at(ex: &mut Explorer, x: i32, y: i32, ctx: &mut Ctx) -> Reply {
    ex.dialog_event(press_at(x, y, 5000), ctx);
    ex.dialog_event(release_at(x, y), ctx)
}

/// Wakes `ex` until `done` holds, for a read let go of on a listing thread. The read can queue
/// behind other tests' reads in the process-wide listing pool, so it gets a whole [`LIST_WAIT`].
pub(crate) fn wake_until(ex: &mut Explorer, ctx: &mut Ctx, done: impl Fn(&Explorer) -> bool) {
    let end = std::time::Instant::now() + LIST_WAIT;
    while !done(ex) {
        assert!(std::time::Instant::now() < end, "the read landed");
        std::thread::sleep(std::time::Duration::from_millis(10));
        ex.wake(ctx);
    }
}

/// Whether `reply` spawns something that opens `path`.
fn opens(reply: &Reply, path: &std::path::Path) -> bool {
    reply
        .spawn
        .iter()
        .any(|argv| argv.iter().any(|a| std::path::Path::new(a) == path))
}
