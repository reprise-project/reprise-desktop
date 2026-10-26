//! Key presses as the bytes a terminal program reads.
//! Sequences ported from alacritty's input handling (Apache-2.0, Copyright The Alacritty Project).

use alacritty_terminal::term::TermMode;
use repriseapp::KeyInput;

/// A key the window keeps for itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shortcut {
    NewTab,
    CloseTab,
    NextTab,
    PrevTab,
    NewWindow,
    Copy,
    Paste,
    Find,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    PageUp,
    PageDown,
}

/// The window's own meaning of `k`, if it has one. Shift+PgUp/PgDn scroll back only outside the
/// alternate screen, where a full-screen program takes them.
pub fn shortcut(k: &KeyInput, mode: TermMode) -> Option<Shortcut> {
    use Shortcut::*;
    let m = k.mods;
    if m.alt {
        return None;
    }
    Some(match (m.ctrl, m.shift, lower(k.sym)) {
        (false, true, PAGE_UP) if !mode.contains(TermMode::ALT_SCREEN) => PageUp,
        (false, true, PAGE_DOWN) if !mode.contains(TermMode::ALT_SCREEN) => PageDown,
        (true, true, 0x74) => NewTab,
        (true, true, 0x77) => CloseTab,
        (true, true, 0x6e) => NewWindow,
        (true, true, 0x63) => Copy,
        (true, true, 0x76) => Paste,
        (true, true, 0x66) => Find,
        (true, false, TAB | PAGE_DOWN) => NextTab,
        (true, true, TAB | ISO_LEFT_TAB) | (true, false, PAGE_UP) => PrevTab,
        (true, _, 0x3d | 0x2b | KP_ADD) => ZoomIn,
        (true, false, 0x2d | KP_SUBTRACT) => ZoomOut,
        (true, false, 0x30 | KP_0) => ZoomReset,
        _ => return None,
    })
}

/// What a press sends; `None` for a key the window keeps (Ctrl+Shift+T, W, C, V, F, N) or one
/// with nothing to send (a modifier alone).
pub fn encode(k: &KeyInput, mode: TermMode) -> Option<Vec<u8>> {
    let text = k.utf8.as_deref().filter(|t| !t.is_empty());
    if k.sym == 0 {
        return text.map(|t| t.as_bytes().to_vec());
    }
    let m = k.mods;
    if m.ctrl && m.shift && matches!(lower(k.sym), 0x63 | 0x66 | 0x6e | 0x74 | 0x76 | 0x77) {
        return None;
    }
    let param = 1 + u8::from(m.shift) + 2 * u8::from(m.alt) + 4 * u8::from(m.ctrl);
    if let Some(seq) = sequence(k.sym, param, mode) {
        return Some(seq);
    }
    let bytes = match k.sym {
        RETURN => vec![b'\r'],
        KP_ENTER => vec![b'\r'],
        TAB if m.shift => return Some(b"\x1b[Z".to_vec()),
        ISO_LEFT_TAB => return Some(b"\x1b[Z".to_vec()),
        TAB => vec![b'\t'],
        BACKSPACE if m.ctrl => vec![0x08],
        BACKSPACE => vec![0x7f],
        ESCAPE => vec![0x1b],
        _ => match m.ctrl.then(|| ctrl_byte(k.sym)).flatten() {
            Some(b) => vec![b],
            None => text
                .filter(|t| !t.chars().any(char::is_control))?
                .as_bytes()
                .to_vec(),
        },
    };
    Some(if m.alt {
        [&[0x1b], &bytes[..]].concat()
    } else {
        bytes
    })
}

/// `text` as a paste, its control characters but tab and line ends dropped: wrapped in the
/// bracketed-paste marks when the program asked for them, with any marks inside cut so the paste
/// cannot end itself; otherwise with each line ended by CR.
pub fn bracket_paste(text: &str, mode: TermMode) -> Vec<u8> {
    let clean = pasteable(text);
    let text = clean.as_ref();
    if mode.contains(TermMode::BRACKETED_PASTE) {
        let inner = match text.contains('\x1b') {
            true => text
                .replace("\x1b[200~", "")
                .replace("\x1b[201~", "")
                .into(),
            false => std::borrow::Cow::Borrowed(text),
        };
        return [b"\x1b[200~", inner.as_bytes(), b"\x1b[201~"].concat();
    }
    let mut out = Vec::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find('\n') {
        let line = &rest[..i];
        out.extend_from_slice(line.as_bytes());
        if !line.ends_with('\r') {
            out.push(b'\r');
        }
        rest = &rest[i + 1..];
    }
    out.extend_from_slice(rest.as_bytes());
    out
}

/// `text` without the C0, DEL and C1 controls, which could start an escape sequence or act as a
/// key in a program that reads the paste as typed input.
pub fn pasteable(text: &str) -> std::borrow::Cow<'_, str> {
    let drop = |c: char| c.is_control() && !matches!(c, '\t' | '\n' | '\r');
    match text.contains(drop) {
        true => text
            .chars()
            .filter(|&c| !drop(c))
            .collect::<String>()
            .into(),
        false => text.into(),
    }
}

const BACKSPACE: u32 = 0xff08;
const TAB: u32 = 0xff09;
const RETURN: u32 = 0xff0d;
const ESCAPE: u32 = 0xff1b;
const ISO_LEFT_TAB: u32 = 0xfe20;
const PAGE_UP: u32 = 0xff55;
const PAGE_DOWN: u32 = 0xff56;
const KP_ADD: u32 = 0xffab;
const KP_SUBTRACT: u32 = 0xffad;
const KP_0: u32 = 0xffb0;
const KP_ENTER: u32 = 0xff8d;
const F1: u32 = 0xffbe;

fn lower(sym: u32) -> u32 {
    if (0x41..=0x5a).contains(&sym) {
        sym + 0x20
    } else {
        sym
    }
}

/// The C0 byte Ctrl makes of `sym`.
fn ctrl_byte(sym: u32) -> Option<u8> {
    Some(match lower(sym) {
        s @ 0x61..=0x7a => (s - 0x60) as u8,
        0x20 | 0x40 | 0x32 => 0x00,
        0x5b | 0x33 => 0x1b,
        0x5c | 0x34 => 0x1c,
        0x5d | 0x35 => 0x1d,
        0x5e | 0x7e | 0x36 => 0x1e,
        0x5f | 0x2f | 0x2d | 0x37 => 0x1f,
        0x3f | 0x38 => 0x7f,
        _ => return None,
    })
}

/// An escape sequence key: cursor and editing keys, F keys, and the keypad in application mode.
/// `param` is xterm's modifier parameter, 1 for none.
fn sequence(sym: u32, param: u8, mode: TermMode) -> Option<Vec<u8>> {
    let cursor = match sym {
        0xff51 | 0xff96 => Some(b'D'),
        0xff52 | 0xff97 => Some(b'A'),
        0xff53 | 0xff98 => Some(b'C'),
        0xff54 | 0xff99 => Some(b'B'),
        0xff50 | 0xff95 => Some(b'H'),
        0xff57 | 0xff9c => Some(b'F'),
        0xff9d => Some(b'E'),
        _ => None,
    };
    if let Some(c) = cursor {
        return Some(if param > 1 {
            format!("\x1b[1;{param}{}", c as char).into_bytes()
        } else if mode.contains(TermMode::APP_CURSOR) {
            vec![0x1b, b'O', c]
        } else {
            vec![0x1b, b'[', c]
        });
    }
    if (F1..F1 + 4).contains(&sym) {
        let c = b"PQRS"[(sym - F1) as usize];
        return Some(if param > 1 {
            format!("\x1b[1;{param}{}", c as char).into_bytes()
        } else {
            vec![0x1b, b'O', c]
        });
    }
    const TILDE_F: [u8; 16] = [
        15, 17, 18, 19, 20, 21, 23, 24, 25, 26, 28, 29, 31, 32, 33, 34,
    ];
    let tilde = match sym {
        0xff63 | 0xff9e => Some(2),
        0xffff | 0xff9f => Some(3),
        0xff55 | 0xff9a => Some(5),
        0xff56 | 0xff9b => Some(6),
        s if (F1 + 4..F1 + 20).contains(&s) => Some(TILDE_F[(s - F1 - 4) as usize]),
        _ => None,
    };
    if let Some(n) = tilde {
        return Some(if param > 1 {
            format!("\x1b[{n};{param}~").into_bytes()
        } else {
            format!("\x1b[{n}~").into_bytes()
        });
    }
    if !mode.contains(TermMode::APP_KEYPAD) {
        return None;
    }
    let keypad = match sym {
        KP_ENTER => b'M',
        0xffaa => b'j',
        0xffab => b'k',
        0xffac => b'l',
        0xffad => b'm',
        0xffae => b'n',
        0xffaf => b'o',
        s @ 0xffb0..=0xffb9 => b'p' + (s - 0xffb0) as u8,
        0xffbd => b'X',
        _ => return None,
    };
    Some(vec![0x1b, b'O', keypad])
}

#[cfg(test)]
mod tests {
    use super::*;
    use repriseapp::Mods;

    const UP: u32 = 0xff52;
    const F1: u32 = 0xffbe;
    const F5: u32 = 0xffc2;
    const KP_1: u32 = 0xffb1;
    const KP_ENTER: u32 = 0xff8d;
    const BACKSPACE: u32 = 0xff08;

    fn key(sym: u32, utf8: Option<&str>, ctrl: bool, shift: bool, alt: bool) -> KeyInput {
        KeyInput {
            sym,
            utf8: utf8.map(str::to_owned),
            mods: Mods { ctrl, shift, alt },
            edit: None,
        }
    }

    fn plain(sym: u32, utf8: Option<&str>) -> KeyInput {
        key(sym, utf8, false, false, false)
    }

    fn none() -> TermMode {
        TermMode::empty()
    }

    #[test]
    fn printable_passes_utf8() {
        assert_eq!(encode(&plain(0x61, Some("a")), none()), Some(b"a".to_vec()));
        let alt_a = key(0x61, Some("a"), false, false, true);
        assert_eq!(encode(&alt_a, none()), Some(b"\x1ba".to_vec()));
        assert_eq!(
            encode(&plain(0xe9, Some("é")), none()),
            Some("é".as_bytes().to_vec())
        );
    }

    #[test]
    fn ctrl_letters() {
        let ctrl_c = key(0x63, Some("\x03"), true, false, false);
        assert_eq!(encode(&ctrl_c, none()), Some(vec![0x03]));
        let ctrl_space = key(0x20, Some(" "), true, false, false);
        assert_eq!(encode(&ctrl_space, none()), Some(vec![0x00]));
        let ctrl_bracket = key(0x5b, Some("\x1b"), true, false, false);
        assert_eq!(encode(&ctrl_bracket, none()), Some(vec![0x1b]));
        let ctrl_alt_x = key(0x78, Some("\x18"), true, false, true);
        assert_eq!(encode(&ctrl_alt_x, none()), Some(vec![0x1b, 0x18]));
    }

    #[test]
    fn cursor_keys_follow_app_cursor() {
        assert_eq!(encode(&plain(UP, None), none()), Some(b"\x1b[A".to_vec()));
        assert_eq!(
            encode(&plain(UP, None), TermMode::APP_CURSOR),
            Some(b"\x1bOA".to_vec())
        );
        let shift_up = key(UP, None, false, true, false);
        assert_eq!(encode(&shift_up, none()), Some(b"\x1b[1;2A".to_vec()));
        assert_eq!(
            encode(&shift_up, TermMode::APP_CURSOR),
            Some(b"\x1b[1;2A".to_vec())
        );
        let ctrl_left = key(0xff51, None, true, false, false);
        assert_eq!(encode(&ctrl_left, none()), Some(b"\x1b[1;5D".to_vec()));
    }

    #[test]
    fn keypad_app_mode() {
        assert_eq!(encode(&plain(KP_1, Some("1")), none()), Some(b"1".to_vec()));
        assert_eq!(
            encode(&plain(KP_1, Some("1")), TermMode::APP_KEYPAD),
            Some(b"\x1bOq".to_vec())
        );
        assert_eq!(
            encode(&plain(KP_ENTER, Some("\r")), none()),
            Some(b"\r".to_vec())
        );
        assert_eq!(
            encode(&plain(KP_ENTER, Some("\r")), TermMode::APP_KEYPAD),
            Some(b"\x1bOM".to_vec())
        );
    }

    #[test]
    fn function_keys() {
        assert_eq!(encode(&plain(F1, None), none()), Some(b"\x1bOP".to_vec()));
        assert_eq!(encode(&plain(F5, None), none()), Some(b"\x1b[15~".to_vec()));
        let ctrl_f5 = key(F5, None, true, false, false);
        assert_eq!(encode(&ctrl_f5, none()), Some(b"\x1b[15;5~".to_vec()));
        let shift_f1 = key(F1, None, false, true, false);
        assert_eq!(encode(&shift_f1, none()), Some(b"\x1b[1;2P".to_vec()));
    }

    #[test]
    fn backspace_is_del() {
        assert_eq!(
            encode(&plain(BACKSPACE, Some("\x08")), none()),
            Some(vec![0x7f])
        );
        let alt_bs = key(BACKSPACE, Some("\x08"), false, false, true);
        assert_eq!(encode(&alt_bs, none()), Some(vec![0x1b, 0x7f]));
        let ctrl_bs = key(BACKSPACE, Some("\x08"), true, false, false);
        assert_eq!(encode(&ctrl_bs, none()), Some(vec![0x08]));
    }

    #[test]
    fn ctrl_shift_reserved() {
        for (sym, ch) in [
            (0x54, "T"),
            (0x57, "W"),
            (0x43, "C"),
            (0x56, "V"),
            (0x46, "F"),
            (0x4e, "N"),
        ] {
            let k = key(sym, Some(ch), true, true, false);
            assert_eq!(encode(&k, none()), None, "Ctrl+Shift+{ch}");
        }
    }

    #[test]
    fn an_input_method_commit_goes_through() {
        assert_eq!(
            encode(&plain(0, Some("日本")), none()),
            Some("日本".as_bytes().to_vec())
        );
    }

    #[test]
    fn bracketed_paste_wraps_and_strips() {
        let text = "a\x1b[201~b\nc";
        assert_eq!(
            bracket_paste(text, TermMode::BRACKETED_PASTE),
            b"\x1b[200~a[201~b\nc\x1b[201~".to_vec(),
            "an end mark inside loses its ESC and cannot end the paste"
        );
        assert_eq!(
            bracket_paste("a\nb\r\nc\r\r\n", none()),
            b"a\rb\rc\r\r".to_vec()
        );
    }

    #[test]
    fn a_paste_loses_its_control_characters() {
        assert_eq!(bracket_paste("a\x1bb\x07c", none()), b"abc".to_vec());
        assert_eq!(
            bracket_paste("a\x1bb\x07c\u{9b}d\x7f\te", TermMode::BRACKETED_PASTE),
            b"\x1b[200~abcd\te\x1b[201~".to_vec()
        );
        assert_eq!(
            bracket_paste("ls\x1b[2K\rrm -rf ~\r", none()),
            b"ls[2K\rrm -rf ~\r".to_vec(),
            "an erase-line cannot hide the second command"
        );
    }

    #[test]
    fn shortcuts() {
        use Shortcut::*;
        let cs = |sym| shortcut(&key(sym, None, true, true, false), none());
        let c = |sym| shortcut(&key(sym, None, true, false, false), none());
        assert_eq!(cs(0x54), Some(NewTab));
        assert_eq!(cs(0x74), Some(NewTab), "lower case from a held Caps Lock");
        assert_eq!(cs(0x57), Some(CloseTab));
        assert_eq!(cs(0x4e), Some(NewWindow));
        assert_eq!(cs(0x43), Some(Copy));
        assert_eq!(cs(0x56), Some(Paste));
        assert_eq!(cs(0x46), Some(Find));
        assert_eq!(c(TAB), Some(NextTab));
        assert_eq!(cs(TAB), Some(PrevTab));
        assert_eq!(cs(ISO_LEFT_TAB), Some(PrevTab));
        assert_eq!(c(0xff55), Some(PrevTab), "Ctrl+PgUp");
        assert_eq!(c(0xff56), Some(NextTab), "Ctrl+PgDn");
        assert_eq!(c(0x3d), Some(ZoomIn));
        assert_eq!(cs(0x2b), Some(ZoomIn), "Ctrl+Shift+= types +");
        assert_eq!(c(0xffab), Some(ZoomIn));
        assert_eq!(c(0x2d), Some(ZoomOut));
        assert_eq!(c(0xffad), Some(ZoomOut));
        assert_eq!(c(0x30), Some(ZoomReset));
        assert_eq!(c(0x74), None, "Ctrl+T goes to the program");
        assert_eq!(shortcut(&plain(0x74, Some("t")), none()), None);
    }

    #[test]
    fn shift_page_keys_scroll_except_on_the_alternate_screen() {
        let sp = |sym, mode| shortcut(&key(sym, None, false, true, false), mode);
        assert_eq!(sp(0xff55, none()), Some(Shortcut::PageUp));
        assert_eq!(sp(0xff56, none()), Some(Shortcut::PageDown));
        assert_eq!(sp(0xff55, TermMode::ALT_SCREEN), None);
        assert_eq!(
            encode(&key(0xff55, None, false, true, false), TermMode::ALT_SCREEN),
            Some(b"\x1b[5;2~".to_vec()),
            "the program gets Shift+PgUp"
        );
    }
}
