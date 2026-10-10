//! Dead keys and the Compose key for the compositor's own text fields: one
//! xkb compose table from the user's locale (and `~/.XCompose`), and a
//! state fed one keysym per press. Clients compose for themselves.

use smithay::input::keyboard::xkb::{self, compose};

use crate::editkeys::KeyText;

pub struct Compose {
    state: compose::State,
}

pub enum Fed {
    Plain,
    /// The press started, continued or cancelled a sequence; it types nothing.
    Swallowed,
    Composed(KeyText),
}

fn locale(var: impl Fn(&str) -> Option<String>) -> String {
    ["LC_ALL", "LC_CTYPE", "LANG"]
        .iter()
        .find_map(|v| var(v).filter(|s| !s.is_empty()))
        .unwrap_or_else(|| "C".into())
}

impl Compose {
    /// None when the locale has no Compose file (every press is then `Fed::Plain`).
    pub fn from_locale() -> Option<Compose> {
        let locale = locale(|v| std::env::var(v).ok());
        if locale.contains('\0') {
            return None;
        }
        let ctx = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        let table =
            compose::Table::new_from_locale(&ctx, locale.as_ref(), compose::COMPILE_NO_FLAGS)
                .ok()?;
        Some(Compose::over(&table))
    }

    fn over(table: &compose::Table) -> Compose {
        Compose {
            state: compose::State::new(table, compose::STATE_NO_FLAGS),
        }
    }

    /// Modifiers leave the sequence as it was.
    pub fn feed(&mut self, sym: u32) -> Fed {
        self.state.feed(sym.into());
        match self.state.status() {
            compose::Status::Nothing => Fed::Plain,
            compose::Status::Composed => {
                let state = self.state.get_raw_ptr();
                let text = KeyText::from_xkb(|buf| {
                    // SAFETY: `state` is live for `self`'s lifetime; `buf` is `buf.len()` writable bytes.
                    unsafe {
                        xkb::ffi::compose::xkb_compose_state_get_utf8(
                            state,
                            buf.as_mut_ptr() as *mut libc::c_char,
                            buf.len(),
                        )
                    }
                });
                self.state.reset();
                text.map_or(Fed::Swallowed, Fed::Composed)
            }
            compose::Status::Cancelled => {
                self.state.reset();
                Fed::Swallowed
            }
            compose::Status::Composing => Fed::Swallowed,
        }
    }

    pub fn reset(&mut self) {
        self.state.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(text: &str) -> Compose {
        let ctx = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        let t = compose::Table::new_from_buffer(
            &ctx,
            text,
            "C",
            compose::FORMAT_TEXT_V1,
            compose::COMPILE_NO_FLAGS,
        )
        .expect("the table compiles");
        Compose::over(&t)
    }

    const ACUTE: &str = "<dead_acute> <e> : \"é\" eacute\n";

    fn fed(c: &mut Compose, sym: u32) -> String {
        match c.feed(sym) {
            Fed::Plain => String::new(),
            Fed::Swallowed => "-".into(),
            Fed::Composed(t) => t.as_str().into(),
        }
    }

    #[test]
    fn a_dead_key_then_a_letter_types_the_composed_letter() {
        let mut c = table(ACUTE);
        assert_eq!(fed(&mut c, crate::keys::KEY_dead_acute), "-");
        assert_eq!(fed(&mut c, crate::keys::KEY_Shift_L), "-");
        assert_eq!(fed(&mut c, crate::keys::KEY_e), "é");
        assert_eq!(fed(&mut c, crate::keys::KEY_e), "");
    }

    #[test]
    fn a_key_outside_the_sequence_cancels_it_and_types_nothing() {
        let mut c = table(ACUTE);
        c.feed(crate::keys::KEY_dead_acute);
        assert_eq!(fed(&mut c, crate::keys::KEY_x), "-");
        assert_eq!(fed(&mut c, crate::keys::KEY_x), "");
    }

    #[test]
    fn reset_drops_a_sequence_in_progress() {
        let mut c = table(ACUTE);
        c.feed(crate::keys::KEY_dead_acute);
        c.reset();
        assert_eq!(fed(&mut c, crate::keys::KEY_e), "");
    }

    #[test]
    fn the_locale_comes_from_lc_all_then_lc_ctype_then_lang() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |v: &str| {
                pairs
                    .iter()
                    .find(|(k, _)| *k == v)
                    .map(|(_, s)| s.to_string())
            }
        };
        assert_eq!(
            locale(env(&[("LANG", "de_DE.UTF-8"), ("LC_CTYPE", "fr_FR.UTF-8")])),
            "fr_FR.UTF-8"
        );
        assert_eq!(
            locale(env(&[("LC_ALL", ""), ("LANG", "de_DE.UTF-8")])),
            "de_DE.UTF-8"
        );
        assert_eq!(locale(env(&[])), "C");
    }
}
