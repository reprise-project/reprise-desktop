//! `[glass] opaque_when_maximized`: an output showing a maximized window draws its taskbar over
//! opaque glass, and back over glass once it shows none.
//!
//! A window counts while it is mapped, maximized or fullscreen, not minimized (itself or up its
//! owner chain), and on the current desktop; it counts for the output it fits ([`State::fit_output`]).
//! Fullscreen counts so a bar revealed over a fullscreen window, or a window leaving fullscreen
//! for maximized, never shows glass in between. Each loop turn ends by recomputing every bar
//! from the windows as they are, so no path that maximizes, minimizes, moves, closes or
//! destroys a window can leave a bar stale, and a bar redraws only on the turn its answer flips.
//! A maximized window's own frame is opaque whenever it is maximized (`glass::frame::maximized`).

use std::borrow::Cow;

use reprisekit::theme::Theme;
use smithay::output::Output;

use crate::compositor::State;
use crate::config::Config;

/// Whether `config` draws bars opaque beside a maximized window: Glass with transparency on and
/// `opaque_when_maximized` (with transparency off every bar is opaque already).
pub fn enabled(config: &Config) -> bool {
    config.glass.opaque_when_maximized && crate::glass::wanted(config)
}

/// The theme a bar draws with: `config`'s, its taskbar over opaque glass when `opaque`.
pub fn bar_theme(config: &Config, opaque: bool) -> Cow<'_, Theme> {
    match opaque {
        true => Cow::Owned(reprisekit::interfaces::opaque_bar(
            &config.theme,
            &config.glass,
            config.taskbar.small_icons,
        )),
        false => Cow::Borrowed(&config.theme),
    }
}

impl State {
    /// The outputs showing a window that makes their bar opaque, by the rule in the module header.
    pub fn maximized_outputs(&self) -> Vec<Output> {
        if !enabled(&self.config) {
            return Vec::new();
        }
        let mut outputs: Vec<Output> = Vec::new();
        for w in &self.windows {
            let shown = w.mapped
                && (w.maximized || w.fullscreen)
                && !self.minimized_along_chain(w.id)
                && self.on_current_desktop(w.id);
            if let Some(o) = shown.then(|| self.fit_output(w.id)).flatten()
                && !outputs.contains(&o)
            {
                outputs.push(o);
            }
        }
        outputs
    }

    /// Bring each bar's opaque state in line with the windows, redrawing the bars that flipped.
    pub fn refresh_opaque_bars(&mut self) {
        let outputs = self.maximized_outputs();
        let mut flipped = Vec::new();
        for bar in &mut self.bars {
            let opaque = outputs.contains(&bar.output);
            if bar.opaque != opaque {
                bar.opaque = opaque;
                bar.drawn = None;
                flipped.push(bar.tree);
            }
        }
        for tree in flipped {
            self.redraw_taskbar(tree);
        }
    }
}

#[cfg(test)]
mod tests;
