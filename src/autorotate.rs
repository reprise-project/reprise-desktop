//! Auto-rotation (`[display] auto_rotate`): while it is on, a thread claims iio-sensor-proxy's
//! accelerometer and reports which way up the machine is held, and the built-in panels turn to
//! keep the picture upright. Turning it off is the rotation lock: the screen stays as it is.

use futures_util::stream::{BoxStream, SelectAll, StreamExt};
use reprisebus::sensorproxy::{self, Orientation};
use smithay::reexports::calloop::{
    LoopHandle,
    channel::{Event, Sender, channel},
};
use smithay::utils::Transform;

use crate::compositor::State;

#[derive(Default)]
pub struct AutoRotate {
    wake: Option<async_channel::Sender<bool>>,
    /// The last orientation the sensor reported, kept while it lies flat.
    held: Option<Orientation>,
}

impl AutoRotate {
    /// Start the thread, claiming the sensor while `on`; it reports on `handle`'s loop.
    pub fn start(handle: &LoopHandle<'static, State>, on: bool) -> AutoRotate {
        let (tx, rx) = channel::<Option<Orientation>>();
        let inserted = handle.insert_source(rx, |event, _, state| {
            if let Event::Msg(Some(o)) = event {
                state.auto_rotate.held = Some(o);
                follow(state);
            }
        });
        if let Err(e) = inserted {
            log::warn!("auto-rotate: channel: {e}");
            return AutoRotate::default();
        }
        let (wake, wakes) = async_channel::unbounded();
        let _ = wake.try_send(on);
        let spawned = std::thread::Builder::new()
            .name("auto-rotate".into())
            .spawn(move || zbus::block_on(run(wakes, tx)));
        if let Err(e) = spawned {
            log::warn!("auto-rotate: no thread: {e}");
            return AutoRotate::default();
        }
        AutoRotate {
            wake: Some(wake),
            held: None,
        }
    }
}

/// The transform that keeps the picture upright on a panel held `o`.
pub fn transform_for(o: Orientation) -> Transform {
    match o {
        Orientation::Normal => Transform::Normal,
        Orientation::LeftUp => Transform::_90,
        Orientation::BottomUp => Transform::_180,
        Orientation::RightUp => Transform::_270,
    }
}

/// Turn the built-in panels the way the machine is held, unless the rotation is locked.
pub fn follow(state: &mut State) {
    if !state.config.auto_rotate {
        return;
    }
    if let Some(o) = state.auto_rotate.held {
        state.turn_panels(transform_for(o));
    }
}

/// An output joined, or a saved layout was put back: turn a panel it brought or reset once the
/// layout has settled.
pub fn outputs_changed(state: &mut State) {
    if state.config.auto_rotate && state.auto_rotate.held.is_some() {
        state.handle.insert_idle(follow);
    }
}

/// Locking keeps the panels as they are turned, saved as their chosen transforms.
pub fn config_changed(state: &mut State) {
    if let Some(tx) = &state.auto_rotate.wake {
        let _ = tx.try_send(state.config.auto_rotate);
    }
    if !state.config.auto_rotate {
        let outputs = state.scene.outputs().to_vec();
        let mut forgot = false;
        for o in &outputs {
            forgot |= crate::output::forget_turn(o);
        }
        if forgot {
            state.save_layout();
        }
    }
}

/// The rotation lock key: lock the screen as it is, or follow the sensor again.
pub fn toggle_lock(state: &mut State) {
    let on = !state.config.auto_rotate;
    state.config.auto_rotate = on;
    let key = repriseconf::key("display", "auto_rotate");
    crate::configwrite::save_flag(&state.config_path, key, on);
    config_changed(state);
    state.show_osd(&crate::media::Report::RotationLock { locked: !on });
}

enum Wake {
    On(bool),
    Proxy(reprisebus::Signal),
}

async fn run(wakes: async_channel::Receiver<bool>, tx: Sender<Option<Orientation>>) {
    let conn = match zbus::Connection::system().await {
        Ok(c) => c,
        Err(e) => {
            log::warn!("auto-rotate: no system bus ({e}); the screen does not turn");
            return;
        }
    };
    let mut events: SelectAll<BoxStream<'static, Wake>> = SelectAll::new();
    events.push(wakes.map(Wake::On).boxed());
    let proxy = reprisebus::signals(&conn, sensorproxy::NAME, sensorproxy::rules()).await;
    events.push(proxy.map(Wake::Proxy).boxed());
    let (mut on, mut claimed) = (false, false);
    while let Some(w) = events.next().await {
        let settle = match w {
            Wake::On(now) => std::mem::replace(&mut on, now) != now,
            // A claim ends with the proxy that took it.
            Wake::Proxy(reprisebus::Signal::OwnerChanged) => {
                claimed = false;
                true
            }
            Wake::Proxy(reprisebus::Signal::Message(m)) => {
                let Some(c) = sensorproxy::change(&m) else {
                    continue;
                };
                if let Some(o) = c.orientation.filter(|_| claimed) {
                    let _ = tx.send(o);
                }
                if c.has_accelerometer == Some(false) {
                    claimed = false;
                }
                c.has_accelerometer.is_some()
            }
        };
        if settle {
            claimed = claim(&conn, on, claimed, &tx).await;
        }
    }
}

/// Hold the sensor while `on` and release it otherwise; whether it is claimed afterwards. A new
/// claim reports the orientation at once.
async fn claim(
    conn: &zbus::Connection,
    on: bool,
    claimed: bool,
    tx: &Sender<Option<Orientation>>,
) -> bool {
    if !on {
        if claimed && let Err(e) = sensorproxy::release(conn).await {
            log::info!("auto-rotate: release: {e}");
        }
        return false;
    }
    if claimed {
        return true;
    }
    if !sensorproxy::has_accelerometer(conn).await {
        log::info!("auto-rotate: no accelerometer reported; the screen does not turn");
        return false;
    }
    if let Err(e) = sensorproxy::claim(conn).await {
        log::warn!("auto-rotate: the accelerometer cannot be claimed: {e}");
        return false;
    }
    log::info!("auto-rotate: following the accelerometer");
    match sensorproxy::orientation(conn).await {
        Ok(o) => {
            let _ = tx.send(o);
        }
        Err(e) => log::info!("auto-rotate: orientation: {e}"),
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_edge_up_turns_the_picture_against_it() {
        assert_eq!(transform_for(Orientation::Normal), Transform::Normal);
        assert_eq!(transform_for(Orientation::LeftUp), Transform::_90);
        assert_eq!(transform_for(Orientation::BottomUp), Transform::_180);
        assert_eq!(transform_for(Orientation::RightUp), Transform::_270);
    }
}
