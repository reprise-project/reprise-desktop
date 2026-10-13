//! The compositor's own tray calls on the session bus: Customize..., Identify, screen saver Preview, tablet pads, and reprise-power's critical battery turn-off.

pub const NAME: &str = "org.reprise.Tray";
pub const PATH: &str = "/org/reprise/Tray";
pub const INTERFACE: &str = "org.reprise.Tray";

/// Ask the compositor to open the tray's Customize dialog. Blocks for at most [`crate::METHOD_TIMEOUT`].
pub fn customize() -> zbus::Result<()> {
    let conn = crate::session()?;
    conn.call_method(Some(NAME), PATH, Some(INTERFACE), "Customize", &())?;
    Ok(())
}

/// Ask the compositor to turn the computer off for a critical battery: it asks programs to close, ends what is left after a few seconds, and runs `[power] poweroff`. Blocks for at most [`crate::METHOD_TIMEOUT`].
pub fn critical_turn_off() -> zbus::Result<()> {
    let conn = crate::session()?;
    conn.call_method(Some(NAME), PATH, Some(INTERFACE), "CriticalTurnOff", &())?;
    Ok(())
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pad {
    /// The device name, as `[[tablet_pads]] name` globs match it.
    pub name: String,
    pub buttons: u32,
    pub rings: u32,
    pub strips: u32,
    pub dials: u32,
    /// The pad keys it has, by the names a `keys` table uses
    /// ("controlpanel").
    pub keys: Vec<String>,
}

/// A [`Pad`] as `TabletPads` sends it.
pub type PadWire = (String, u32, u32, u32, u32, Vec<String>);

impl Pad {
    pub fn wire(&self) -> PadWire {
        (
            self.name.clone(),
            self.buttons,
            self.rings,
            self.strips,
            self.dials,
            self.keys.clone(),
        )
    }

    pub fn from_wire((name, buttons, rings, strips, dials, keys): PadWire) -> Pad {
        Pad {
            name,
            buttons,
            rings,
            strips,
            dials,
            keys,
        }
    }
}

/// The tablet pads connected now, in the order they came. Blocks for at most [`crate::METHOD_TIMEOUT`].
pub fn tablet_pads() -> zbus::Result<Vec<Pad>> {
    let conn = crate::session()?;
    let reply = conn.call_method(Some(NAME), PATH, Some(INTERFACE), "TabletPads", &())?;
    let list: Vec<PadWire> = reply.body().deserialize()?;
    Ok(list.into_iter().map(Pad::from_wire).collect())
}

/// Ask the compositor to show each output's number, big in its middle for 2 s. Blocks for at most [`crate::METHOD_TIMEOUT`].
pub fn identify(outputs: &[String]) -> zbus::Result<()> {
    let conn = crate::session()?;
    conn.call_method(Some(NAME), PATH, Some(INTERFACE), "Identify", &(outputs,))?;
    Ok(())
}

/// Ask the compositor to show the screen saver `name` full screen until the next input. Blocks for at most [`crate::METHOD_TIMEOUT`].
pub fn preview_screen_saver(name: &str) -> zbus::Result<()> {
    let conn = crate::session()?;
    conn.call_method(
        Some(NAME),
        PATH,
        Some(INTERFACE),
        "PreviewScreenSaver",
        &(name,),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::zvariant::Type;

    #[test]
    fn a_pad_crosses_the_bus_with_its_dials_and_keys() {
        assert_eq!(PadWire::SIGNATURE.to_string(), "(suuuuas)");
        let pad = Pad {
            name: "Wacom Intuos Pro M Pad".into(),
            buttons: 9,
            rings: 1,
            strips: 0,
            dials: 2,
            keys: vec!["controlpanel".into(), "onscreen_keyboard".into()],
        };
        assert_eq!(Pad::from_wire(pad.wire()), pad);
    }
}
