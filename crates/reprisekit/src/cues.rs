//! Keyboard cues: the dotted focus rectangles and the access keys'
//! underlines. A window opened with the pointer hides both and one opened
//! from the keyboard shows both. Any key brings out the rectangles; Alt,
//! alone or held with a key, brings out the underlines too. The rectangles
//! stay for the window's life, the underlines until it loses the keyboard.
//! A host hands `keys` to the draw as [`crate::theme::Theme::access_keys`].

use std::borrow::Cow;

use crate::text::{escape_mnemonic, split_mnemonic};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cues {
    pub focus: bool,
    pub keys: bool,
}

impl Cues {
    pub const NONE: Cues = Cues {
        focus: false,
        keys: false,
    };
    pub const ALL: Cues = Cues {
        focus: true,
        keys: true,
    };

    pub fn opened(by_key: bool) -> Cues {
        if by_key { Cues::ALL } else { Cues::NONE }
    }

    /// The window was asked to open while already open: from the keyboard it
    /// shows both cues, as a fresh keyboard open does. Whether a cue came out.
    pub fn reopen(&mut self, by_key: bool) -> bool {
        by_key && self.key(true)
    }

    /// A key went down; `alt` when it is Alt or Alt is held with it.
    /// Whether a cue came out.
    pub fn key(&mut self, alt: bool) -> bool {
        let before = *self;
        self.focus = true;
        self.keys |= alt;
        *self != before
    }

    /// The window lost the keyboard: the underlines hide. Whether they were out.
    pub fn blur(&mut self) -> bool {
        std::mem::take(&mut self.keys)
    }

    /// `label` as drawn: without its access key's marker while the
    /// underlines hide (`&&` stays a literal ampersand).
    pub fn label<'a>(&self, label: &'a str) -> Cow<'a, str> {
        if self.keys || !label.contains('&') {
            Cow::Borrowed(label)
        } else {
            Cow::Owned(escape_mnemonic(&split_mnemonic(label).0))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_shows_the_rectangles_and_alt_the_underlines() {
        let mut c = Cues::opened(false);
        assert_eq!(c, Cues::NONE);
        assert!(c.key(false), "a key brings out the rectangles");
        assert_eq!(
            c,
            Cues {
                focus: true,
                keys: false
            }
        );
        assert!(!c.key(false), "a second key changes nothing");
        assert!(c.key(true), "Alt brings out the underlines");
        assert_eq!(c, Cues::ALL);
        assert!(!c.key(false), "and they stay");
        assert_eq!(Cues::opened(true), Cues::ALL);
    }

    #[test]
    fn opening_an_open_window_from_the_keyboard_shows_both_cues() {
        let mut blurred = Cues::ALL;
        blurred.blur();
        assert!(blurred.reopen(true), "the underlines come back");
        assert_eq!(blurred, Cues::ALL);
        assert!(!blurred.reopen(true), "nothing left to show");
        let mut pointer = Cues::opened(false);
        assert!(!pointer.reopen(false), "a click keeps the cues hidden");
        assert_eq!(pointer, Cues::NONE);
        assert!(pointer.reopen(true));
        assert_eq!(pointer, Cues::ALL);
    }

    #[test]
    fn losing_the_keyboard_hides_the_underlines_and_keeps_the_rectangles() {
        let mut c = Cues::ALL;
        assert!(c.blur(), "the underlines were out");
        assert_eq!(
            c,
            Cues {
                focus: true,
                keys: false
            }
        );
        assert!(!c.blur(), "nothing left to hide");
        assert!(c.key(true), "Alt brings them back");
    }

    #[test]
    fn hidden_underlines_drop_the_marker_and_keep_a_literal_ampersand() {
        assert_eq!(Cues::NONE.label("&Browse..."), "Browse...");
        assert_eq!(Cues::NONE.label("Cut && &Paste"), "Cut && Paste");
        assert_eq!(Cues::ALL.label("&Browse..."), "&Browse...");
        assert_eq!(split_mnemonic(&Cues::NONE.label("A && &B")).1, None);
    }
}
