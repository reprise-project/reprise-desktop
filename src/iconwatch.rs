//! Keeps `State::icons` current: a thread warms the icon theme off the loop, then waits for a
//! directory it reads to change, and the loop reloads the theme and redraws what shows icons.
use crate::compositor::State;
use reprisefs::dirchange::{Stop, Woke};
use reprisekit::icontheme::IconTheme;
use smithay::reexports::calloop::channel::{self, Event, Sender};
use std::sync::{Arc, Weak};
use std::time::Duration;

/// One package install is one reload.
const SETTLE: Duration = Duration::from_millis(500);

/// The loop's end of the watch, and the [`Stop`] of the thread watching the current theme.
#[derive(Default)]
pub struct IconWatch {
    tx: Option<Sender<Weak<IconTheme>>>,
    stop: Option<Stop>,
}

/// Join the watch's channel to the loop and watch the theme. Runs once, after `State::new`.
pub fn setup(state: &mut State) {
    let (tx, rx) = channel::channel::<Weak<IconTheme>>();
    let joined = state.handle.insert_source(rx, |ev, _, state| {
        if let Event::Msg(watched) = ev {
            changed(state, &watched);
        }
    });
    match joined {
        Ok(_) => state.icon_watch.tx = Some(tx),
        Err(e) => log::warn!("icon theme watch: {}", e.error),
    }
    watch(state);
}

/// Looks names up in `theme` from now on, redraws what shows icons and watches the new theme.
pub fn apply(state: &mut State, theme: IconTheme) {
    state.icons.set_theme(theme);
    // Which icon name a desktop file wears depends on what the theme has.
    state.rescan_desktop();
    state.invalidate_desktop();
    state.invalidate_dialogs();
    let name = state.config.icons.theme.clone();
    crate::tray::send(state, crate::tray::TrayCommand::IconTheme(name));
    watch(state);
}

fn watch(state: &mut State) {
    // The thread watching the theme before this one ends now.
    state.icon_watch.stop = None;
    let Some(tx) = state.icon_watch.tx.clone() else {
        return;
    };
    let (stop, stopped) = match reprisefs::dirchange::stop() {
        Ok(pair) => pair,
        Err(e) => {
            log::warn!("icon theme watch: {e}");
            return;
        }
    };
    state.icon_watch.stop = Some(stop);
    let theme = state.icons.theme();
    let spawned = std::thread::Builder::new()
        .name("icon-watch".into())
        .spawn(move || {
            theme.warm();
            let dirs = theme.watch_dirs();
            let watched = Arc::downgrade(&theme);
            drop(theme);
            if let Ok(Woke::Changed) = reprisefs::dirchange::wait(&dirs, SETTLE, &stopped) {
                let _ = tx.send(watched);
            }
        });
    if let Err(e) = spawned {
        log::warn!("icon theme watch: {e}");
    }
}

/// A directory of `watched` changed: read the theme again unless another replaced it since.
fn changed(state: &mut State, watched: &Weak<IconTheme>) {
    let current = state.icons.theme();
    if !watched.upgrade().is_some_and(|t| Arc::ptr_eq(&t, &current)) {
        return;
    }
    log::info!("icon theme changed on disk, reloading");
    apply(state, current.reload());
    crate::tray::invalidate_icons(state);
    crate::configwatch::redraw_for_scale(state);
    state.redraw_taskbars();
    state.relook_menu_icons();
    for depth in 0..state.menus.len() {
        state.redraw_menu(depth);
    }
}
