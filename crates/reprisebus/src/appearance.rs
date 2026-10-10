//! The look a running program follows, read through the Settings portal under Reprise's
//! own namespace. A program starts with the look the compositor put in its environment;
//! these keys carry every change after that.

use zbus::blocking::Proxy;
use zbus::blocking::proxy::Builder;
use zbus::proxy::CacheProperties;
use zbus::zvariant::OwnedValue;

/// The namespace the compositor's Settings backend serves the look in.
pub const NAMESPACE: &str = "org.reprise.appearance";
/// `classic`, `blue` or `glass`, as `REPRISE_INTERFACE`.
pub const INTERFACE: &str = "interface";
/// Blue's colour scheme, `blue`, `olive` or `silver`, as
/// `REPRISE_BLUE_VARIANT`.
pub const BLUE_VARIANT: &str = "blue-variant";
/// The effective colour scheme, as `REPRISE_COLORS`.
pub const COLORS: &str = "colors";
/// The icon theme's name, as `REPRISE_ICONS`.
pub const ICON_THEME: &str = "icon-theme";
/// `[fonts] shell_dlg`, a family or a font file's path; empty when unset.
pub const SHELL_DLG: &str = "shell-dlg";

/// `[glass] mode`, `light` or `dark`, as `REPRISE_GLASS_MODE`.
pub const GLASS_MODE: &str = "glass-mode";

/// `[accessibility] text_size`, in percent ("150").
pub const TEXT_SIZE: &str = "text-size";

const KEYS: [&str; 7] = [
    INTERFACE,
    BLUE_VARIANT,
    GLASS_MODE,
    COLORS,
    ICON_THEME,
    SHELL_DLG,
    TEXT_SIZE,
];

/// Call `changed(key, value)` from a thread of its own for each key of
/// [`NAMESPACE`]: once with the current value when the watch starts, then
/// on every `SettingChanged`. A session with no portal gets no calls.
pub fn watch(mut changed: impl FnMut(&str, String) + Send + 'static) {
    let run = move || {
        if let Err(e) = follow(&mut changed) {
            log::info!("appearance: {e}; the look stays as the program started");
        }
    };
    if let Err(e) = std::thread::Builder::new()
        .name("appearance".into())
        .spawn(run)
    {
        log::warn!("appearance thread: {e}");
    }
}

fn follow(changed: &mut impl FnMut(&str, String)) -> zbus::Result<()> {
    let conn = crate::session()?;
    let proxy: Proxy = Builder::new(&conn)
        .destination("org.freedesktop.portal.Desktop")?
        .path("/org/freedesktop/portal/desktop")?
        .interface("org.freedesktop.portal.Settings")?
        .cache_properties(CacheProperties::No)
        .build()?;
    // Subscribed before the first read, so a change in between is not lost.
    let signals = proxy.receive_signal("SettingChanged")?;
    for key in KEYS {
        match proxy.call::<_, _, OwnedValue>("ReadOne", &(NAMESPACE, key)) {
            Ok(v) => {
                if let Some(s) = string(v) {
                    changed(key, s);
                }
            }
            Err(e) => log::debug!("appearance: ReadOne {key}: {e}"),
        }
    }
    for msg in signals {
        let Ok((namespace, key, value)) = msg.body().deserialize::<(String, String, OwnedValue)>()
        else {
            continue;
        };
        if namespace == NAMESPACE
            && KEYS.contains(&key.as_str())
            && let Some(s) = string(value)
        {
            changed(&key, s);
        }
    }
    Ok(())
}

fn string(v: OwnedValue) -> Option<String> {
    String::try_from(v).ok()
}
