//! The lock's now-playing strip: the player's title over its artist, then Previous, Play or Pause
//! and Next at its right end. Each interface picks where the strip goes and inks its text; the
//! buttons are drawn by `interfaces::logon`. The strip shows nothing of the player but the two
//! lines, and a press only asks the host for [`LogonAction::Media`](super::LogonAction::Media).

use super::Part;
use crate::media::{NowPlaying, Transport};
use crate::raster::Rect;
use crate::theme::TextUnits;

/// Design px at 100% text, each growing with it.
const W: i32 = 320;
const H: i32 = 34;
const ROW_H: i32 = 13;
const BUTTON_W: i32 = 32;
const BUTTON_H: i32 = 28;
const BUTTON_GAP: i32 = 4;
const LABEL_GAP: i32 = 8;
/// The most characters of a title or artist the strip keeps.
pub const TEXT_MAX: usize = 200;

/// The strip's size where the interface does not size it.
pub fn size(u: TextUnits) -> (i32, i32) {
    (u.len(W), u.len(H))
}

/// The buttons inside `strip`: at its right end, centred down it.
pub fn buttons(u: TextUnits, strip: Rect) -> [(Part, Rect); 3] {
    let (bw, bh, gap) = (u.len(BUTTON_W), u.len(BUTTON_H), u.len(BUTTON_GAP));
    let x = strip.right() - (3 * bw + 2 * gap);
    let y = strip.y + (strip.height - bh) / 2;
    Transport::ALL.map(|t| {
        let at = x + t as i32 * (bw + gap);
        (Part::Media(t), Rect::new(at, y, bw, bh))
    })
}

/// The lines `strip` shows and where: the title over the artist left of the buttons, or the
/// title alone, centred down it.
pub fn lines(np: &NowPlaying, u: TextUnits, strip: Rect) -> Vec<(Rect, &str)> {
    let (bw, gap, row) = (u.len(BUTTON_W), u.len(BUTTON_GAP), u.len(ROW_H));
    let w = strip.width - (3 * bw + 2 * gap) - u.len(LABEL_GAP);
    let shown: Vec<&str> = [np.title.as_str(), np.artist.as_str()]
        .into_iter()
        .filter(|l| !l.is_empty())
        .collect();
    let top = strip.y + (strip.height - shown.len() as i32 * row) / 2;
    (shown.into_iter().enumerate())
        .map(|(i, l)| (Rect::new(strip.x, top + i as i32 * row, w, row), l))
        .collect()
}

/// `s` as the strip keeps it: no control characters, at most [`TEXT_MAX`] characters.
pub fn clean(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control())
        .take(TEXT_MAX)
        .collect()
}

/// What a reader hears for the strip's text.
pub fn label(np: &NowPlaying) -> String {
    match np.artist.is_empty() {
        true => np.title.clone(),
        false => format!("{}, {}", np.title, np.artist),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::access::Tree;
    use crate::access::accesskit::Role;
    use crate::interface::Interface;
    use crate::interfaces::logon::logon_layout;
    use crate::logon::{Logon, LogonAction, LogonKey, LogonUser, access, hit};
    use crate::pointer::Interaction;
    use crate::text::TextRenderer;

    const OUTPUT: Rect = Rect::new(0, 0, 1024, 768);

    pub(crate) fn playing() -> NowPlaying {
        NowPlaying {
            title: "Night Drive".into(),
            artist: "The Examples".into(),
            playing: true,
            can_previous: true,
            can_next: true,
            can_play: true,
            can_pause: true,
        }
    }

    fn user() -> LogonUser {
        LogonUser {
            name: "anna".into(),
            real_name: "Anna Novak".into(),
            picture: None,
            note: None,
            stock: 0,
        }
    }

    fn lock(i: Interface, np: Option<NowPlaying>) -> Logon {
        let mut m = Logon::locked(i, user(), true, true);
        m.set_media(np);
        m
    }

    /// A lock with no player, one playing, one paused that can't go back with Play focused and
    /// the pointer on Next, and one naming no artist.
    pub(crate) fn states(i: Interface) -> Vec<(&'static str, Logon)> {
        let paused = NowPlaying {
            playing: false,
            can_previous: false,
            ..playing()
        };
        let mut keys = lock(i, Some(paused));
        keys.focus = Part::Media(Transport::PlayPause);
        keys.lit = Some((Part::Media(Transport::Next), Interaction::Hot));
        let alone = NowPlaying {
            artist: String::new(),
            ..playing()
        };
        vec![
            ("none", lock(i, None)),
            ("playing", lock(i, Some(playing()))),
            ("paused play focused next hot", keys),
            ("title alone", lock(i, Some(alone))),
        ]
    }

    fn laid_out(m: &Logon) -> crate::logon::LogonLayout {
        let theme = crate::interfaces::theme_for(m.interface);
        let text = TextRenderer::load(&theme.fonts);
        logon_layout(m, &theme, &text, OUTPUT)
    }

    #[test]
    fn three_32_px_buttons_4_px_apart_end_the_strip_centred_down_it() {
        let strip = Rect::new(100, 50, 320, 34);
        let b = buttons(TextUnits::ONE, strip).map(|(_, r)| r);
        assert_eq!(
            b,
            [
                Rect::new(316, 53, 32, 28),
                Rect::new(352, 53, 32, 28),
                Rect::new(388, 53, 32, 28),
            ]
        );
        assert_eq!(b[2].right(), strip.right());
    }

    #[test]
    fn the_title_stands_over_the_artist_or_alone_in_the_middle() {
        fn lines(np: &NowPlaying) -> Vec<(Rect, &str)> {
            super::lines(np, TextUnits::ONE, Rect::new(0, 0, 320, 34))
        }
        assert_eq!(
            lines(&playing()),
            [
                (Rect::new(0, 4, 208, 13), "Night Drive"),
                (Rect::new(0, 17, 208, 13), "The Examples")
            ],
            "the text stops 8 px short of the buttons"
        );
        let alone = NowPlaying {
            artist: String::new(),
            ..playing()
        };
        assert_eq!(lines(&alone), [(Rect::new(0, 10, 208, 13), "Night Drive")]);
    }

    #[test]
    fn the_strip_keeps_no_control_characters_and_200_characters_at_most() {
        assert_eq!(clean("a\nb\u{1b}[2Jc"), "ab[2Jc");
        assert_eq!(clean(&"x".repeat(500)).chars().count(), TEXT_MAX);
        let mut m = lock(Interface::Classic, None);
        m.set_media(Some(NowPlaying {
            title: "line\none".into(),
            ..playing()
        }));
        assert_eq!(m.media.unwrap().title, "lineone");
    }

    #[test]
    fn a_login_screen_shows_no_player() {
        let mut m = Logon::new(Interface::Glass, vec![user()], true, None);
        m.set_media(Some(playing()));
        assert_eq!(m.media, None);
        assert!(
            !laid_out(&m)
                .parts
                .iter()
                .any(|(p, _)| matches!(p, Part::Media(_)))
        );
    }

    #[test]
    fn the_password_keeps_the_focus_and_tab_reaches_the_buttons_the_player_can_press_last() {
        for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
            let bare = lock(i, None).stops();
            let mut m = lock(
                i,
                Some(NowPlaying {
                    can_previous: false,
                    ..playing()
                }),
            );
            assert_eq!(m.focus, Part::Password, "{i:?}");
            let mut stops = bare.clone();
            stops.extend([
                Part::Media(Transport::PlayPause),
                Part::Media(Transport::Next),
            ]);
            assert_eq!(
                m.stops(),
                stops,
                "{i:?}: the screen's own stops, then Pause and Next"
            );
            m.focus = *bare.last().unwrap();
            m.key(LogonKey::Tab { back: false });
            assert_eq!(m.focus, Part::Media(Transport::PlayPause));
            assert_eq!(
                m.key(LogonKey::Enter),
                Some(LogonAction::Media(Transport::PlayPause))
            );
            assert_eq!(m.key(LogonKey::Text("x")), None, "typing presses nothing");
            assert_eq!(m.password.text, "", "nor reaches the password");
            m.key(LogonKey::Escape);
            assert_eq!(m.focus, Part::Password, "Escape goes back to the password");
        }
    }

    #[test]
    fn a_click_presses_a_button_the_player_can_press_and_leaves_the_focus() {
        let mut m = lock(
            Interface::Classic,
            Some(NowPlaying {
                can_previous: false,
                ..playing()
            }),
        );
        assert_eq!(m.click(Part::Media(Transport::Previous)), None, "greyed");
        assert_eq!(
            m.click(Part::Media(Transport::Next)),
            Some(LogonAction::Media(Transport::Next))
        );
        assert_eq!(m.focus, Part::Password);
        m.set_busy(true);
        assert_eq!(
            m.click(Part::Media(Transport::PlayPause)),
            Some(LogonAction::Media(Transport::PlayPause)),
            "a check in progress leaves the player alone"
        );
    }

    #[test]
    fn a_button_that_goes_or_greys_gives_the_focus_back_to_the_password() {
        let mut m = lock(Interface::Glass, Some(playing()));
        m.focus = Part::Media(Transport::Previous);
        m.lit = Some((Part::Media(Transport::Previous), Interaction::Hot));
        m.set_media(Some(NowPlaying {
            can_previous: false,
            ..playing()
        }));
        assert_eq!((m.focus, m.lit), (Part::Password, None), "greyed");
        m.focus = Part::Media(Transport::Next);
        m.set_media(None);
        assert_eq!(m.focus, Part::Password, "gone");
    }

    #[test]
    fn each_look_puts_the_strip_on_the_output_clear_of_the_screens_other_parts() {
        for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
            let m = lock(i, Some(playing()));
            let l = laid_out(&m);
            let strip = l.media.expect("a strip");
            assert_eq!(OUTPUT.union(&strip), OUTPUT, "{i:?}: {strip:?}");
            let (media, rest): (Vec<_>, Vec<_>) = l
                .parts
                .iter()
                .copied()
                .partition(|(p, _)| matches!(p, Part::Media(_)));
            assert_eq!(media.len(), 3, "{i:?}");
            for (p, r) in &media {
                assert_eq!(strip.union(r), strip, "{i:?}: {p:?} in the strip");
                let (x, y) = (r.x + r.width / 2, r.y + r.height / 2);
                assert_eq!(hit(&l, x, y), Some(*p), "{i:?}");
            }
            for (p, r) in &rest {
                assert!(
                    !r.intersects(strip),
                    "{i:?}: {p:?} {r:?} clear of {strip:?}"
                );
            }
            assert!(
                l.dialog.is_none_or(|d| !d.intersects(strip)),
                "{i:?}: clear of the dialog"
            );
            let bare = laid_out(&lock(i, None));
            assert_eq!((bare.media, bare.parts), (None, rest));
        }
    }

    #[test]
    fn a_reader_hears_the_title_and_artist_and_each_button_by_what_it_does() {
        let mut m = lock(
            Interface::Blue,
            Some(NowPlaying {
                playing: false,
                can_next: false,
                ..playing()
            }),
        );
        m.focus = Part::Media(Transport::PlayPause);
        let l = laid_out(&m);
        let mut tree = Tree::window("Unlock", (OUTPUT.width, OUTPUT.height));
        access(&m, &l, &mut tree);
        let play = tree
            .find(Role::Button, "Play")
            .expect("Play, as it is paused");
        assert_eq!(tree.focus(), play);
        let next = tree.find(Role::Button, "Next").unwrap();
        assert!(tree.get(next).unwrap().is_disabled(), "Next greys");
        let text = (tree.nodes()).find(|(_, n)| n.value() == Some("Night Drive, The Examples"));
        assert!(text.is_some_and(|(_, n)| n.role() == Role::Label));
    }
}
