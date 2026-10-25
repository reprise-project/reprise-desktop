//! The compositor's own tray calls on the session bus: Customize..., Identify, screen saver Preview, tablet pads, the window icon a sound program borrows, and reprise-power's critical battery turn-off.

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

/// The icon of the window a program plays sound from, as `WindowIcon` finds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WindowIcon {
    /// A theme icon name, or an icon file's absolute path.
    Named(String),
    /// The window's own icon at its native size, rows of premultiplied ARGB.
    Pixels {
        width: u32,
        height: u32,
        pixels: Vec<u32>,
    },
}

/// What `WindowIcon` answers: (name, width, height, pixels), a name for [`WindowIcon::Named`],
/// else the pixels; all empty when no window matches.
pub type WindowIconWire = (String, u32, u32, Vec<u32>);

impl WindowIcon {
    pub fn wire(found: Option<WindowIcon>) -> WindowIconWire {
        match found {
            None => (String::new(), 0, 0, Vec::new()),
            Some(WindowIcon::Named(n)) => (n, 0, 0, Vec::new()),
            Some(WindowIcon::Pixels {
                width,
                height,
                pixels,
            }) => (String::new(), width, height, pixels),
        }
    }

    pub fn from_wire((name, width, height, pixels): WindowIconWire) -> Option<WindowIcon> {
        if !name.is_empty() {
            return Some(WindowIcon::Named(name));
        }
        let fits =
            width > 0 && height > 0 && pixels.len() as u64 == u64::from(width) * u64::from(height);
        fits.then_some(WindowIcon::Pixels {
            width,
            height,
            pixels,
        })
    }
}

/// The icon of the window whose process is one of `pids` (host pids, best first) or an ancestor
/// of one, else of the window titled `title`. Blocks for at most [`crate::METHOD_TIMEOUT`].
pub fn window_icon(pids: &[u32], title: &str) -> zbus::Result<Option<WindowIcon>> {
    let conn = crate::session()?;
    let reply = conn.call_method(
        Some(NAME),
        PATH,
        Some(INTERFACE),
        "WindowIcon",
        &(pids, title),
    )?;
    Ok(WindowIcon::from_wire(reply.body().deserialize()?))
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

    #[test]
    fn a_window_icon_crosses_the_bus_as_a_name_or_pixels() {
        assert_eq!(WindowIconWire::SIGNATURE.to_string(), "(suuau)");
        for icon in [
            None,
            Some(WindowIcon::Named("brave-origin".into())),
            Some(WindowIcon::Pixels {
                width: 2,
                height: 1,
                pixels: vec![0xff00_0000, 0x8080_8080],
            }),
        ] {
            assert_eq!(WindowIcon::from_wire(WindowIcon::wire(icon.clone())), icon);
        }
        assert_eq!(
            WindowIcon::from_wire((String::new(), 2, 2, vec![0])),
            None,
            "pixels short of width by height"
        );
    }
}
