//! `reprisekit::interfaces` with every theme built at the gallery's text size, so the whole gallery
//! draws at 200% for `clip_test`. The pages take their themes from here.

use reprisekit::interface::Interface;
pub use reprisekit::interfaces::*;
use reprisekit::theme::Theme;
use reprisekit::{blue, glass};
use std::cell::Cell;

thread_local! {
    static TEXT_SIZE: Cell<u32> = const { Cell::new(100) };
}

/// Builds this thread's gallery themes at `pct` percent text from here on.
#[cfg_attr(not(test), allow(dead_code))]
pub fn set_text_size(pct: u32) {
    TEXT_SIZE.set(pct);
}

fn sized(t: Theme) -> Theme {
    t.with_text_size(TEXT_SIZE.get())
}

/// Classic's theme, `Theme::default()` at the gallery's text size.
pub fn classic() -> Theme {
    sized(Theme::default())
}

pub fn theme_for(i: Interface) -> Theme {
    sized(reprisekit::interfaces::theme_for(i))
}

pub fn theme_with(
    i: Interface,
    variant: blue::Variant,
    dark: bool,
    glass: &glass::Settings,
) -> Theme {
    sized(reprisekit::interfaces::theme_with(i, variant, dark, glass))
}

pub fn blue_theme(v: blue::Variant) -> Theme {
    sized(reprisekit::interfaces::blue_theme(v))
}

pub fn blue_theme_in(v: blue::Variant, dark: bool) -> Theme {
    sized(reprisekit::interfaces::blue_theme_in(v, dark))
}
