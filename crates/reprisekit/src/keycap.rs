//! Keysyms by their xkb names, and a key as its cap reads.

use std::ffi::{CString, c_char, c_int};

const XKB_KEYSYM_CASE_INSENSITIVE: c_int = 1;

#[link(name = "xkbcommon")]
unsafe extern "C" {
    fn xkb_keysym_from_name(name: *const c_char, flags: c_int) -> u32;
    fn xkb_keysym_get_name(keysym: u32, buffer: *mut c_char, size: usize) -> c_int;
    fn xkb_keysym_to_lower(keysym: u32) -> u32;
    fn xkb_keysym_to_utf32(keysym: u32) -> u32;
}

pub fn key_known(name: &str) -> bool {
    let Ok(c) = CString::new(name) else {
        return false;
    };
    // SAFETY: `c` is a NUL-terminated string that outlives the call.
    unsafe { xkb_keysym_from_name(c.as_ptr(), XKB_KEYSYM_CASE_INSENSITIVE) != 0 }
}

pub fn key_name(sym: u32) -> Option<String> {
    let mut buf = [0 as c_char; 64];
    // SAFETY: xkb writes at most `buf.len()` bytes, NUL included, and
    // returns how many it wanted; -1 for an invalid keysym.
    let n = unsafe { xkb_keysym_get_name(xkb_keysym_to_lower(sym), buf.as_mut_ptr(), buf.len()) };
    if n <= 0 || n as usize >= buf.len() {
        return None;
    }
    // SAFETY: xkb NUL-terminated the name inside `buf`.
    let name = unsafe { std::ffi::CStr::from_ptr(buf.as_ptr()) };
    name.to_str().ok().map(str::to_string)
}

pub fn keysym(name: &str) -> Option<u32> {
    let c = CString::new(name).ok()?;
    // SAFETY: `c` is a NUL-terminated string that outlives the call;
    // `xkb_keysym_to_lower` is a pure function on a keysym value.
    let sym = unsafe {
        xkb_keysym_to_lower(xkb_keysym_from_name(
            c.as_ptr(),
            XKB_KEYSYM_CASE_INSENSITIVE,
        ))
    };
    (sym != 0).then_some(sym)
}

/// Keysym `sym` as a key cap names it: a letter in capitals, a typed sign as
/// itself, a named key in words ("Page Down", "Print").
pub fn key_text(sym: u32) -> String {
    if let Some(c) = char::from_u32(sym).filter(|_| (0x21..=0x7e).contains(&sym)) {
        return c.to_ascii_uppercase().to_string();
    }
    let name = key_name(sym).unwrap_or_else(|| format!("{sym:#x}"));
    let fixed = match name.as_str() {
        "Return" | "return" => "Enter",
        "space" => "Space",
        "Escape" | "escape" => "Esc",
        "Sys_Req" | "sys_req" => "SysRq",
        "Prior" => "Page Up",
        "Next" => "Page Down",
        "Super_L" | "super_l" => "Left Super",
        "Super_R" | "super_r" => "Right Super",
        "Alt_L" | "alt_l" => "Left Alt",
        "Alt_R" | "alt_r" => "Right Alt",
        "Shift_L" | "shift_l" => "Left Shift",
        "Shift_R" | "shift_r" => "Right Shift",
        "Control_L" | "control_l" => "Left Ctrl",
        "Control_R" | "control_r" => "Right Ctrl",
        _ => "",
    };
    if !fixed.is_empty() {
        return fixed.to_string();
    }
    let name = name.strip_prefix("XF86").unwrap_or(&name);
    // Underscores and case steps split words: "Page_Down", "AudioRaiseVolume".
    let mut out = String::new();
    let mut prev: Option<char> = None;
    for c in name.chars() {
        if c == '_' {
            out.push(' ');
        } else {
            if c.is_uppercase() && prev.is_some_and(char::is_lowercase) {
                out.push(' ');
            }
            out.push(c);
        }
        prev = Some(c);
    }
    let mut chars = out.chars();
    chars
        .next()
        .map(|f| f.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

/// Whether keysym `sym` types a character.
pub fn types_char(sym: u32) -> bool {
    // SAFETY: a pure function on a keysym value.
    unsafe { xkb_keysym_to_utf32(sym) != 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_names_are_the_lower_case_keysyms() {
        assert_eq!(key_name(0x42).as_deref(), Some("b"));
        assert_eq!(key_name(0x21).as_deref(), Some("exclam"));
        assert_eq!(key_name(0xff0d).as_deref(), Some("Return"));
        assert_eq!(key_name(0xffe9).as_deref(), Some("Alt_L"));
        assert_eq!(
            key_name(0x1008ff13).as_deref(),
            Some("XF86AudioRaiseVolume")
        );
    }

    #[test]
    fn caps_name_letters_signs_and_named_keys() {
        assert_eq!(key_text(0x62), "B");
        assert_eq!(key_text(0x3d), "=");
        assert_eq!(key_text(0xff0d), "Enter");
        assert_eq!(key_text(0xff55), "Page Up");
        assert_eq!(key_text(0xffe9), "Left Alt");
        assert_eq!(key_text(0x1008ff13), "Audio Raise Volume");
    }
}
