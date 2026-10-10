use reprisekit::textedit::{EditKey, key_for};
use smithay::input::keyboard::{KeyboardHandle, Keycode, xkb};
use zeroize::Zeroize;

use crate::compositor::State;

/// Held on the stack and zeroed on drop, so a password's characters leave no
/// copy behind.
pub struct KeyText {
    buf: [u8; 64],
    len: usize,
}

impl KeyText {
    /// Like snprintf, `fill` returns the full length, NUL excluded, so a result
    /// that does not fit is None rather than cut mid-character.
    pub fn from_xkb(fill: impl FnOnce(&mut [u8]) -> i32) -> Option<KeyText> {
        let mut t = KeyText {
            buf: [0; 64],
            len: 0,
        };
        let len = usize::try_from(fill(&mut t.buf)).ok()?;
        if len == 0 || len >= t.buf.len() || std::str::from_utf8(&t.buf[..len]).is_err() {
            return None;
        }
        t.len = len;
        Some(t)
    }

    pub fn of_key(state: &xkb::State, code: Keycode) -> Option<KeyText> {
        KeyText::from_xkb(|buf| {
            // SAFETY: `state` is a live xkb state for the call; `buf` is
            // `buf.len()` writable bytes.
            unsafe {
                xkb::ffi::xkb_state_key_get_utf8(
                    state.get_raw_ptr(),
                    code.raw(),
                    buf.as_mut_ptr() as *mut libc::c_char,
                    buf.len(),
                )
            }
        })
    }

    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.buf[..self.len]).unwrap_or_default()
    }
}

impl Drop for KeyText {
    fn drop(&mut self) {
        self.buf.zeroize();
    }
}

pub fn from_keyboard(
    keyboard: &KeyboardHandle<State>,
    data: &mut State,
    code: Keycode,
) -> Option<EditKey> {
    let mods = keyboard.modifier_state();
    keyboard.with_xkb_state(data, |ctx| {
        let x = ctx.xkb().lock().unwrap();
        // SAFETY: the state is only read; no reference outlives the lock.
        let state = unsafe { x.state() };
        let text = KeyText::of_key(state, code);
        let utf8 = text.as_ref().map(KeyText::as_str);
        (state.key_get_syms(code).iter())
            .find_map(|s| key_for(s.raw(), mods.ctrl, mods.shift, utf8))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn xkb_writes(s: &str) -> impl FnOnce(&mut [u8]) -> i32 {
        move |buf: &mut [u8]| {
            let n = s.len().min(buf.len() - 1);
            buf[..n].copy_from_slice(&s.as_bytes()[..n]);
            buf[n] = 0;
            s.len() as i32
        }
    }

    #[test]
    fn key_text_holds_what_fits() {
        let t = KeyText::from_xkb(xkb_writes("é")).expect("fits");
        assert_eq!(t.as_str(), "é");
        let t = KeyText::from_xkb(xkb_writes(&"é".repeat(31))).expect("62 bytes fit");
        assert_eq!(t.as_str(), "é".repeat(31));
    }

    #[test]
    fn key_text_drops_a_result_that_does_not_fit_or_is_empty() {
        assert!(KeyText::from_xkb(xkb_writes(&"é".repeat(32))).is_none());
        assert!(KeyText::from_xkb(xkb_writes("")).is_none());
        assert!(KeyText::from_xkb(|_| -1).is_none());
    }
}
