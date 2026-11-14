//! The refresh rate of the output recorded, from `wl_output`.

use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_output, wl_registry};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, WEnum};

/// The current mode's refresh of the `wl_output` named `output`, in whole hertz.
pub fn refresh_hz(conn: &Connection, output: &str) -> Option<u32> {
    let (globals, mut queue) = registry_queue_init::<Outputs>(conn).ok()?;
    let qh = queue.handle();
    let mut state = Outputs::default();
    for g in (globals.contents().clone_list().iter())
        .filter(|g| g.interface == wl_output::WlOutput::interface().name && g.version >= 4)
    {
        let out: wl_output::WlOutput = globals.registry().bind(g.name, 4, &qh, ());
        state.outputs.push((out, String::new(), 0));
    }
    queue.roundtrip(&mut state).ok()?;
    let found = (state.outputs.iter()).find(|(_, name, _)| name == output);
    let hz = found
        .map(|&(_, _, mhz)| mhz_to_hz(mhz))
        .filter(|&hz| hz > 0);
    for (out, _, _) in state.outputs {
        out.release();
    }
    hz
}

fn mhz_to_hz(mhz: i32) -> u32 {
    (mhz.max(0) as u32 + 500) / 1000
}

#[derive(Default)]
struct Outputs {
    /// Each output, its name and its current mode's refresh in mHz.
    outputs: Vec<(wl_output::WlOutput, String, i32)>,
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Outputs {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_output::WlOutput, ()> for Outputs {
    fn event(
        state: &mut Self,
        out: &wl_output::WlOutput,
        event: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some((_, name, refresh)) = state.outputs.iter_mut().find(|(o, _, _)| o == out) else {
            return;
        };
        match event {
            wl_output::Event::Name { name: n } => *name = n,
            wl_output::Event::Mode {
                refresh: r,
                flags: WEnum::Value(f),
                ..
            } if f.contains(wl_output::Mode::Current) => *refresh = r,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn millihertz_round_to_whole_hertz() {
        assert_eq!(
            [59_940, 60_000, 143_856, 0, -1].map(mhz_to_hz),
            [60, 60, 144, 0, 0]
        );
    }
}
