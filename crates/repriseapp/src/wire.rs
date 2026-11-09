//! Text a program hands the compositor in a Wayland request. The compositor reads each request
//! into a 4096-byte buffer and disconnects a client whose request does not fit.

/// The most bytes of a title, app id or tag sent in one request.
pub(crate) const TEXT_MAX: usize = 1024;

/// `s` cut to [`TEXT_MAX`] bytes on a char boundary.
pub(crate) fn text(s: &str) -> &str {
    let mut end = s.len().min(TEXT_MAX);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_text_goes_whole() {
        assert_eq!(text("Notes - Editor"), "Notes - Editor");
    }

    #[test]
    fn a_long_text_is_cut_to_the_byte_cap_on_a_char_boundary() {
        let astral = "\u{1F600}".repeat(1024);
        let cut = text(&astral);
        assert_eq!(
            cut.len(),
            TEXT_MAX,
            "256 four-byte chars fill the cap exactly"
        );
        let skewed = format!("a{astral}");
        assert_eq!(
            text(&skewed).len(),
            1 + 255 * 4,
            "a char straddling the cap is left out whole"
        );
    }
}
