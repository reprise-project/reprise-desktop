//! The look a running program follows, read through the Settings portal under Reprise's
//! own namespace. A program starts with the look the compositor put in its environment;
//! these keys carry every change after that.

use zbus::OwnedVariant;
use zbus::Proxy;
use zbus::proxy::{Builder, CacheProperties};

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
/// `[fonts] family`, the families in order joined by commas ([`families`] splits them).
pub const FONT_FAMILY: &str = "font-family";
/// `[fonts] antialias`, `true` or `false`.
pub const FONT_ANTIALIAS: &str = "font-antialias";

/// `[glass] mode`, `light` or `dark`, as `REPRISE_GLASS_MODE`.
pub const GLASS_MODE: &str = "glass-mode";

/// `[glass] material`, `frosted`, `smoked` or `molten`, as `REPRISE_GLASS_MATERIAL`.
pub const GLASS_MATERIAL: &str = "glass-material";

/// `[accessibility] text_size`, in percent ("150").
pub const TEXT_SIZE: &str = "text-size";

/// `[metrics] double_click_ms` in force, in ms ("500").
pub const DOUBLE_CLICK_MS: &str = "double-click-ms";

const KEYS: [&str; 11] = [
    INTERFACE,
    BLUE_VARIANT,
    GLASS_MODE,
    GLASS_MATERIAL,
    COLORS,
    ICON_THEME,
    SHELL_DLG,
    FONT_FAMILY,
    FONT_ANTIALIAS,
    TEXT_SIZE,
    DOUBLE_CLICK_MS,
];

/// The families a [`FONT_FAMILY`] value names, in order; none when it is empty.
pub fn families(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|f| !f.is_empty())
        .map(String::from)
        .collect()
}

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
    let proxy: Proxy = crate::wait(
        Builder::new(&conn)
            .destination("org.freedesktop.portal.Desktop")
            .path("/org/freedesktop/portal/desktop")
            .interface("org.freedesktop.portal.Settings")
            .cache_properties(CacheProperties::No)
            .build(),
    )?;
    // Subscribed before the first read, so a change in between is not lost.
    let signals = crate::wait(proxy.receive_signal("SettingChanged"))?;
    for key in KEYS {
        match crate::wait(proxy.call::<_, _, OwnedVariant>("ReadOne", &(NAMESPACE, key))) {
            Ok(v) => {
                if let Some(s) = string(v) {
                    changed(key, s);
                }
            }
            Err(e) => log::debug!("appearance: ReadOne {key}: {e}"),
        }
    }
    for msg in crate::blocking_iter(signals) {
        let Ok((namespace, key, value)) =
            msg.body().deserialize::<(String, String, OwnedVariant)>()
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

fn string(v: OwnedVariant) -> Option<String> {
    String::try_from(v).ok()
}
