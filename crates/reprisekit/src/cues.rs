//! Keyboard cues: the dotted focus rectangles and the access keys'
//! underlines. A window opened with the pointer hides both and one opened
//! from the keyboard shows both. Any key brings out the rectangles; Alt,
//! alone or held with a key, brings out the underlines too. Once out, a
//! cue stays for the window's life.

use std::borrow::Cow;

use crate::text::split_mnemonic;

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

    /// A key went down; `alt` when it is Alt or Alt is held with it.
    /// Whether a cue came out.
    pub fn key(&mut self, alt: bool) -> bool {
        let before = *self;
        self.focus = true;
        self.keys |= alt;
        *self != before
    }

    /// `label` as drawn: without its access key's marker while the
    /// underlines hide (`&&` stays a literal ampersand).
    pub fn label<'a>(&self, label: &'a str) -> Cow<'a, str> {
        if self.keys || !label.contains('&') {
            Cow::Borrowed(label)
        } else {
            Cow::Owned(split_mnemonic(label).0.replace('&', "&&"))
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
    fn hidden_underlines_drop_the_marker_and_keep_a_literal_ampersand() {
        assert_eq!(Cues::NONE.label("&Browse..."), "Browse...");
        assert_eq!(Cues::NONE.label("Cut && &Paste"), "Cut && Paste");
        assert_eq!(Cues::ALL.label("&Browse..."), "&Browse...");
        assert_eq!(split_mnemonic(&Cues::NONE.label("A && &B")).1, None);
    }
}
