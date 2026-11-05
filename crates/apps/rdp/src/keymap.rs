//! Client keys as the session's: RDP scancodes (set 1 and the extended flag) to evdev codes,
//! Unicode key events to keysyms, and the client's lock keys.

/// The evdev code of set-1 scancode `code`, `extended` when it came with the 0xE0 prefix.
pub fn evdev(code: u8, extended: bool) -> Option<u32> {
    let key = if extended {
        match code {
            0x10 => 165, // previous song
            0x19 => 163, // next song
            0x1C => 96,  // keypad Enter
            0x1D => 97,  // right Ctrl
            0x20 => 113, // mute
            0x21 => 140, // calculator
            0x22 => 164, // play/pause
            0x24 => 166, // stop
            0x2E => 114, // volume down
            0x30 => 115, // volume up
            0x32 => 172, // browser home
            0x35 => 98,  // keypad /
            0x36 => 54,  // right Shift
            0x37 => 99,  // Print Screen
            0x38 => 100, // right Alt
            0x45 => 69,  // Num Lock
            0x46 => 119, // Pause
            0x47 => 102,
            0x48 => 103,
            0x49 => 104,
            0x4B => 105,
            0x4D => 106,
            0x4F => 107,
            0x50 => 108,
            0x51 => 109,
            0x52 => 110,
            0x53 => 111,
            0x5B => 125, // left Super
            0x5C => 126, // right Super
            0x5D => 127, // Menu
            0x5E => 116, // power
            0x5F => 142, // sleep
            0x63 => 143, // wake up
            0x65 => 217, // search
            0x66 => 156, // bookmarks
            0x67 => 173, // refresh
            0x68 => 128, // stop
            0x69 => 159, // forward
            0x6A => 158, // back
            0x6C => 155, // mail
            0x6D => 226, // media
            _ => return None,
        }
    } else {
        match code {
            0x01..=0x53 => u32::from(code),
            0x54 => 99, // Alt+Print Screen
            0x56..=0x58 => u32::from(code),
            0x59 => 117,                         // keypad =
            0x64..=0x6E => u32::from(code) + 83, // F13 to F23
            0x70 => 93,                          // Katakana/Hiragana
            0x71 => 123,                         // Hanja
            0x72 => 122,                         // Hangul
            0x73 => 89,                          // Ro
            0x76 => 194,                         // F24
            0x79 => 92,                          // Henkan
            0x7B => 94,                          // Muhenkan
            0x7D => 124,                         // Yen
            0x7E => 121,                         // keypad comma
            _ => return None,
        }
    };
    Some(key)
}

const PAUSE: u32 = 119;

/// The client's scancode events as evdev keys. Pause comes as E1 1D then 45, pressed and
/// released alike, and is one key.
#[derive(Default)]
pub struct Scancodes {
    /// The last event was E1 1D, so a 45 now ends Pause.
    in_pause: bool,
}

impl Scancodes {
    /// The key of one scancode event; None for no key and for the 45 that ends Pause.
    pub fn key(&mut self, code: u8, extended: bool, extended1: bool) -> Option<u32> {
        if std::mem::take(&mut self.in_pause) && code == 0x45 && !extended && !extended1 {
            return None;
        }
        if extended1 {
            self.in_pause = code == 0x1D;
            return self.in_pause.then_some(PAUSE);
        }
        evdev(code, extended)
    }
}

/// Joins a client's UTF-16 key events into code points.
#[derive(Default)]
pub struct Utf16 {
    high: Option<u16>,
}

impl Utf16 {
    /// The keysym of the code point `unit` completes; a lone low surrogate is dropped.
    pub fn push(&mut self, unit: u16) -> Option<u32> {
        let high = self.high.take();
        let point = match unit {
            0xD800..=0xDBFF => {
                self.high = Some(unit);
                return None;
            }
            0xDC00..=0xDFFF => {
                let high = high?;
                0x10000 + ((u32::from(high) - 0xD800) << 10) + (u32::from(unit) - 0xDC00)
            }
            _ => u32::from(unit),
        };
        keysym(point)
    }
}

/// The keysym typing `point`: Latin-1 as itself, a control key as its key's keysym, the rest
/// as a Unicode keysym.
fn keysym(point: u32) -> Option<u32> {
    match point {
        0x08 | 0x09 | 0x0D | 0x1B => Some(0xFF00 | point),
        0x7F => Some(0xFFFF),
        0x00..=0x1F | 0x80..=0x9F => None,
        0x20..=0xFF => Some(point),
        _ => Some(0x0100_0000 | point),
    }
}

/// Lock keys that are on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Locks {
    pub caps: bool,
    pub num: bool,
    pub scroll: bool,
}

const CAPS_LOCK: u32 = 58;
const NUM_LOCK: u32 = 69;
const SCROLL_LOCK: u32 = 70;

/// The evdev keys to tap so that `current` becomes `wanted`.
pub fn sync_toggles(current: Locks, wanted: Locks) -> Vec<u32> {
    [
        (current.caps != wanted.caps, CAPS_LOCK),
        (current.num != wanted.num, NUM_LOCK),
        (current.scroll != wanted.scroll, SCROLL_LOCK),
    ]
    .into_iter()
    .filter_map(|(differs, key)| differs.then_some(key))
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scancodes_map_to_evdev() {
        assert_eq!(evdev(0x1E, false), Some(30), "A");
        assert_eq!(evdev(0x1D, true), Some(97), "right Ctrl");
        assert_eq!(evdev(0x38, true), Some(100), "right Alt");
        assert_eq!(evdev(0x48, true), Some(103), "Up");
        assert_eq!(evdev(0x53, true), Some(111), "Delete");
        assert_eq!(evdev(0x5B, true), Some(125), "left Super");
        assert_eq!(evdev(0x5D, true), Some(127), "Menu");
        assert_eq!(evdev(0x1C, true), Some(96), "keypad Enter");
        assert_eq!(evdev(0x45, false), Some(69), "Num Lock");
        assert_eq!(evdev(0x56, false), Some(86), "the key left of Z");
        assert_eq!(evdev(0x7F, true), None);
    }

    #[test]
    fn function_keys_f13_to_f24_map_to_their_evdev_codes() {
        assert_eq!(evdev(0x64, false), Some(183), "F13");
        assert_eq!(evdev(0x6E, false), Some(193), "F23");
        assert_eq!(evdev(0x76, false), Some(194), "F24");
        assert_eq!(evdev(0x6F, false), None, "no key");
    }

    #[test]
    fn a_surrogate_pair_becomes_one_keysym() {
        let mut utf16 = Utf16::default();
        assert_eq!(utf16.push(0xD83D), None);
        assert_eq!(utf16.push(0xDE00), Some(0x0101_F600));
    }

    #[test]
    fn latin1_maps_to_itself() {
        assert_eq!(Utf16::default().push(0xE9), Some(0xE9));
    }

    #[test]
    fn a_lone_low_surrogate_is_dropped() {
        let mut utf16 = Utf16::default();
        assert_eq!(utf16.push(0xDE00), None);
        assert_eq!(utf16.push(0x61), Some(0x61), "the next key types");
    }

    #[test]
    fn sync_taps_only_what_differs() {
        let current = Locks {
            caps: true,
            num: true,
            scroll: false,
        };
        let wanted = Locks {
            caps: false,
            ..current
        };
        assert_eq!(sync_toggles(current, wanted), vec![58]);
    }
}
