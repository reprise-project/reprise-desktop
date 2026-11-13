//! What Welcome shares with the sheet: its picks read from and written into `config.toml` through
//! the sheet's model, and the Appearance page's sample of them.

use crate::appearancepage;
use crate::model::{self, Settings};
use reprisefs::apps::AppEntry;
use reprisekit::controls::welcome::{Picks, TEXT_SIZES, dark_offered};
use reprisekit::interface::Interface;
use reprisekit::schemes;
use reprisekit::theme::Theme;
use std::path::Path;

/// The Classic scheme Dark picks.
pub const DARK_SCHEME: &str = "One Dark";

fn dark(s: &Settings) -> bool {
    match appearancepage::interface(s) {
        Interface::Classic => schemes::scheme(&s.color_scheme).is_some_and(|c| c.is_dark()),
        Interface::Blue => false,
        Interface::Glass => s.glass_mode == "dark",
    }
}

/// The picks `text` holds; a text size off the steps reads as 100%.
pub fn read(text: &str) -> Picks {
    let s = model::load(text);
    let size = u32::try_from(s.text_size).unwrap_or(100);
    Picks {
        interface: appearancepage::interface(&s),
        dark: dark(&s),
        text_size: if TEXT_SIZES.contains(&size) {
            size
        } else {
            100
        },
    }
}

/// `text`'s settings, and them with each pick that differs laid over, a new interface as the
/// Interface combo moves it; a Glass "night" or "hours" mode stays until Light or Dark moves.
fn with(text: &str, picks: &Picks) -> (Settings, Settings) {
    let base = model::load(text);
    let mut s = base.clone();
    let i = picks.interface;
    if appearancepage::interface(&s) != i {
        let from = std::mem::replace(&mut s.interface, i.as_str().into());
        model::switch_interface(&mut s, &from, text, &base);
    }
    if dark_offered(i) && dark(&s) != picks.dark {
        match i {
            Interface::Classic if picks.dark => s.color_scheme = DARK_SCHEME.into(),
            Interface::Classic => s.color_scheme.clear(),
            Interface::Glass => s.glass_mode = if picks.dark { "dark" } else { "light" }.into(),
            Interface::Blue => {}
        }
    }
    s.text_size = picks.text_size.into();
    (base, s)
}

/// Desktop entry `id` as the sheet's link rows find it: `None` when it is not installed or not shown.
pub fn entry(id: &str) -> Option<AppEntry> {
    crate::linkhost::find(id, &crate::linkhost::dirs())
}

/// The Appearance page's sample of `text` with `picks` written.
pub fn sample(text: &str, picks: &Picks) -> Theme {
    appearancepage::theme(&with(text, picks).1)
}

/// Write `picks` into the config file at `path` as Apply would, touching only the keys they move.
pub fn write(path: &Path, picks: &Picks) -> Result<(), String> {
    let written = repriseconf::edit(path, |text| {
        let (base, want) = with(text, picks);
        model::save(text, &want, &base)
    });
    match written {
        Ok(r) => r.map(|_| ()),
        Err(e) => Err(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picks(interface: Interface, dark: bool, text_size: u32) -> Picks {
        Picks {
            interface,
            dark,
            text_size,
        }
    }

    fn written(text: &str, p: Picks) -> String {
        let (base, want) = with(text, &p);
        model::save(text, &want, &base).unwrap()
    }

    #[test]
    fn an_empty_file_reads_as_classic_light_at_100() {
        assert_eq!(read(""), picks(Interface::Classic, false, 100));
    }

    #[test]
    fn unmoved_picks_write_nothing() {
        let text = "[appearance]\ninterface = \"glass\"\n\n[glass]\nmode = \"night\"\n";
        assert_eq!(written(text, read(text)), text, "night stays night");
    }

    #[test]
    fn each_pick_writes_its_key() {
        let glass = written("", picks(Interface::Glass, true, 150));
        assert_eq!(read(&glass), picks(Interface::Glass, true, 150), "{glass}");
        assert!(glass.contains("interface = \"glass\""), "{glass}");
        assert!(glass.contains("mode = \"dark\""), "{glass}");
        assert!(glass.contains("text_size = 150"), "{glass}");

        let classic = written("", picks(Interface::Classic, true, 100));
        assert!(
            classic.contains(&format!("color_scheme = \"{DARK_SCHEME}\"")),
            "{classic}"
        );
        assert_eq!(read(&classic), picks(Interface::Classic, true, 100));
        let light = written(&classic, picks(Interface::Classic, false, 100));
        assert!(
            !light.contains("color_scheme"),
            "Light is the default scheme: {light}"
        );
    }

    #[test]
    fn a_new_interface_drops_the_old_ones_scheme() {
        let text = format!("[appearance]\ncolor_scheme = \"{DARK_SCHEME}\"\n");
        let blue = written(&text, picks(Interface::Blue, false, 100));
        assert!(!blue.contains(DARK_SCHEME), "{blue}");
        assert_eq!(read(&blue), picks(Interface::Blue, false, 100));
    }

    #[test]
    fn the_sample_shows_the_picks() {
        assert!(
            sample("", &picks(Interface::Glass, false, 100))
                .glass()
                .is_some()
        );
        assert!(
            sample("", &picks(Interface::Classic, false, 100))
                .glass()
                .is_none()
        );
        let dark = sample("", &picks(Interface::Classic, true, 100));
        assert!(dark.colors.is_dark(), "{DARK_SCHEME} is a dark scheme");
    }
}
