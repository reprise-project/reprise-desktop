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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pad {
    /// The device name, as `[[tablet_pads]] name` globs match it.
    pub name: String,
    pub buttons: u32,
    pub rings: u32,
    pub strips: u32,
}

/// The tablet pads connected now, in the order they came. Blocks for at most [`crate::METHOD_TIMEOUT`].
pub fn tablet_pads() -> zbus::Result<Vec<Pad>> {
    let conn = crate::session()?;
    let reply = conn.call_method(Some(NAME), PATH, Some(INTERFACE), "TabletPads", &())?;
    let list: Vec<(String, u32, u32, u32)> = reply.body().deserialize()?;
    Ok(list
        .into_iter()
        .map(|(name, buttons, rings, strips)| Pad {
            name,
            buttons,
            rings,
            strips,
        })
        .collect())
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
