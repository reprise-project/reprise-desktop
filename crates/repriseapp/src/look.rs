//! The theme, fonts and icons a program draws with, followed through the portal.

use crate::{App, Event, Host, WinId};
use reprisekit::icontheme::{IconCache, IconTheme};
use reprisekit::text::TextRenderer;
use reprisekit::theme::Theme;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

thread_local! {
    /// The host's theme and the main window's output width, for [`caption_fit`].
    pub(crate) static CAPTION_FIT: RefCell<(Option<Theme>, Option<i32>)> =
        const { RefCell::new((None, None)) };
}

/// The narrowest client width at which a child window's caption shows `title` whole under the
/// host's theme (an owned child wears the dialog caption), no wider than the main window's
/// output allows. 0 before the host has a theme.
pub fn caption_fit(title: &str) -> i32 {
    CAPTION_FIT.with_borrow(|(theme, _)| {
        let Some(theme) = theme else {
            return 0;
        };
        let fit = reprisekit::look::caption_fit(theme, title, reprisekit::frame::Chrome::DIALOG);
        match dialog_max_w() {
            0 => fit,
            max => fit.min(max),
        }
    })
}

/// The widest client width a child dialog's frame leaves on the main window's output, for
/// `msgbox::Fields::max_w`. 0 before the host knows its theme and output.
pub fn dialog_max_w() -> i32 {
    use reprisekit::frame::{Chrome, FrameGeometry};
    CAPTION_FIT.with_borrow(|(theme, output_w)| match (theme, output_w) {
        (Some(theme), Some(w)) => {
            FrameGeometry::client_size_for_total(&theme.metrics, *w, 1, false, Chrome::DIALOG).0
        }
        _ => 0,
    })
}

/// `theme` at `text_size` for an [`Extended::text_sized`] program, else as it is.
pub(crate) fn sized(theme: Theme, app: &dyn App, text_size: u32) -> Theme {
    if app.text_sized() {
        theme.with_text_size(text_size)
    } else {
        theme
    }
}

pub(crate) fn publish_theme(theme: &Theme) {
    CAPTION_FIT.with_borrow_mut(|(t, _)| *t = Some(theme.clone()));
}

/// The theme a program builds from the compositor's environment: the
/// interface named by [`reprisekit::interface::ENV`] (Blue variant from
/// [`reprisekit::blue::ENV`], Glass mode and material from
/// [`reprisekit::glass::MODE_ENV`] and [`reprisekit::glass::MATERIAL_ENV`]), else
/// [`Theme::default`]; then a
/// [`reprisekit::theme::COLORS_ENV`] scheme is laid over it.
pub(crate) fn theme_from_env(
    interface: Option<&str>,
    variant: Option<&str>,
    glass_mode: Option<&str>,
    glass_material: Option<&str>,
    colors: Option<&str>,
) -> Theme {
    let mut theme = reprisekit::interfaces::theme_with(
        reprisekit::interface::Interface::from_env_value(interface),
        reprisekit::blue::Variant::from_env_value(variant),
        &reprisekit::glass::Settings {
            mode: glass_mode
                .and_then(reprisekit::glass::Mode::parse)
                .unwrap_or_default(),
            material: glass_material
                .and_then(reprisekit::glass::Material::parse)
                .unwrap_or_default(),
            ..reprisekit::glass::Settings::default()
        },
    );
    if let Some(s) = colors
        && let Some(cs) = reprisekit::theme::ColorScheme::from_env_over(theme.colors.clone(), s)
    {
        theme.colors = cs;
    }
    theme
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Look {
    interface: Option<String>,
    blue_variant: Option<String>,
    glass_mode: Option<String>,
    glass_material: Option<String>,
    colors: Option<String>,
    icon_theme: String,
    /// `[fonts] shell_dlg`, read from `config.toml` at the start and
    /// followed through the portal after: the second dialog font every
    /// renderer loads beside the UI font.
    shell_dlg: Option<String>,
    /// `[fonts] family`, read and followed as `shell_dlg` is; `None` keeps the interface's own.
    font_family: Option<Vec<String>>,
    /// `[fonts] antialias`, read and followed as `shell_dlg` is; `None` keeps the interface's own.
    antialias: Option<bool>,
    /// `[accessibility] text_size` in percent, read and followed as
    /// `shell_dlg` is, for [`Ctx::text_size`](crate::Ctx::text_size).
    pub(crate) text_size: u32,
    /// `[metrics] double_click_ms`, read and followed as `shell_dlg` is; `None` keeps the
    /// interface's own.
    double_click_ms: Option<i32>,
}

impl Look {
    pub(crate) fn from_env() -> Look {
        let keys = repriseconf::look_keys();
        Look {
            interface: std::env::var(reprisekit::interface::ENV).ok(),
            blue_variant: std::env::var(reprisekit::blue::ENV).ok(),
            glass_mode: std::env::var(reprisekit::glass::MODE_ENV).ok(),
            glass_material: std::env::var(reprisekit::glass::MATERIAL_ENV).ok(),
            colors: std::env::var(reprisekit::theme::COLORS_ENV).ok(),
            icon_theme: std::env::var(reprisekit::icontheme::ENV)
                .unwrap_or_else(|_| "hicolor".into()),
            shell_dlg: keys.shell_dlg,
            font_family: keys.font_family,
            antialias: keys.antialias,
            text_size: keys.text_size,
            double_click_ms: keys.double_click_ms,
        }
    }

    pub(crate) fn theme(&self) -> Theme {
        let mut theme = theme_from_env(
            self.interface.as_deref(),
            self.blue_variant.as_deref(),
            self.glass_mode.as_deref(),
            self.glass_material.as_deref(),
            self.colors.as_deref(),
        );
        theme.fonts.shell_dlg = self.shell_dlg.clone();
        if let Some(family) = &self.font_family {
            theme.fonts.family = family.clone();
        }
        if let Some(antialias) = self.antialias {
            theme.fonts.antialias = antialias;
        }
        if let Some(ms) = self.double_click_ms {
            theme.metrics.double_click_ms = ms;
        }
        theme
    }

    pub(crate) fn icons(&self) -> IconCache {
        IconCache::new(IconTheme::load(
            &self.icon_theme,
            &IconTheme::default_roots(),
        ))
    }

    pub(crate) fn set(&mut self, key: &str, value: String) -> bool {
        use reprisebus::appearance as a;
        if key == a::SHELL_DLG {
            let font = Some(value).filter(|s| !s.trim().is_empty());
            let changed = self.shell_dlg != font;
            self.shell_dlg = font;
            return changed;
        }
        if key == a::FONT_FAMILY {
            let family = Some(a::families(&value)).filter(|f| !f.is_empty());
            let changed = self.font_family != family;
            self.font_family = family;
            return changed;
        }
        if key == a::FONT_ANTIALIAS {
            let Ok(antialias) = value.parse() else {
                return false;
            };
            let changed = self.antialias != Some(antialias);
            self.antialias = Some(antialias);
            return changed;
        }
        if key == a::DOUBLE_CLICK_MS {
            let Ok(ms) = value.parse() else {
                return false;
            };
            let changed = self.double_click_ms != Some(ms);
            self.double_click_ms = Some(ms);
            return changed;
        }
        if key == a::TEXT_SIZE {
            let Ok(size) = value.parse() else {
                return false;
            };
            let changed = self.text_size != size;
            self.text_size = size;
            return changed;
        }
        let field = match key {
            a::INTERFACE => self.interface.get_or_insert_default(),
            a::BLUE_VARIANT => self.blue_variant.get_or_insert_default(),
            a::GLASS_MODE => self.glass_mode.get_or_insert_default(),
            a::GLASS_MATERIAL => self.glass_material.get_or_insert_default(),
            a::COLORS => self.colors.get_or_insert_default(),
            a::ICON_THEME => &mut self.icon_theme,
            _ => return false,
        };
        let changed = *field != value;
        *field = value;
        changed
    }
}

/// The theme `run` starts in, which a dialog built before `run` sizes itself for.
pub fn start_theme() -> Theme {
    Look::from_env().theme()
}

thread_local! {
    pub(crate) static TEXT: RefCell<Option<(reprisekit::theme::FontConfig, Rc<TextRenderer>)>> =
        const { RefCell::new(None) };
}

/// A renderer for `fonts`, the one loaded last when the fonts are the same.
pub(crate) fn text_for(fonts: &reprisekit::theme::FontConfig) -> Rc<TextRenderer> {
    TEXT.with_borrow_mut(|held| match held {
        Some((f, text)) if f == fonts => text.clone(),
        _ => {
            let text = Rc::new(TextRenderer::load(fonts));
            *held = Some((fonts.clone(), text.clone()));
            text
        }
    })
}

/// [`Ctx::text`](crate::Ctx::text) as `run` starts, for a renderer a program needs before its first
/// event.
pub fn start_text() -> Rc<TextRenderer> {
    text_for(&start_theme().fonts)
}

/// [`Ctx::forms`](crate::Ctx::forms) as the program starts, for an [`App::initial_size`] asked
/// before any event.
pub fn start_forms() -> Theme {
    let look = Look::from_env();
    look.theme().with_form_text(look.text_size)
}

impl Host {
    /// Warms the icon theme off the loop, then reloads it once a directory it reads changes, so
    /// an icon installed or removed while the program runs shows.
    pub(crate) fn watch_icons(&mut self) {
        self.icon_watch = None;
        let (stop, stopped) = match reprisefs::dirchange::stop() {
            Ok(pair) => pair,
            Err(e) => {
                log::warn!("icon theme watch: {e}");
                return;
            }
        };
        self.icon_watch = Some(stop);
        let theme = self.ctx.icons.theme();
        let tx = self.portal_done.clone();
        let spawned = std::thread::Builder::new()
            .name("icon-watch".into())
            .spawn(move || {
                theme.warm();
                let dirs = theme.watch_dirs();
                let watched = Arc::downgrade(&theme);
                drop(theme);
                let woke = reprisefs::dirchange::wait(&dirs, ICONS_SETTLE, &stopped);
                if let Ok(reprisefs::dirchange::Woke::Changed) = woke {
                    let _ = tx.send(Box::new(move |host: &mut Host| {
                        host.icons_changed(&watched)
                    }));
                }
            });
        if let Err(e) = spawned {
            log::warn!("icon theme watch: {e}");
        }
    }

    fn icons_changed(&mut self, watched: &std::sync::Weak<IconTheme>) {
        let current = self.ctx.icons.theme();
        if !watched.upgrade().is_some_and(|t| Arc::ptr_eq(&t, &current)) {
            return;
        }
        self.ctx.icons.reload();
        for s in self.slots.iter_mut() {
            s.dirty = true;
        }
        self.watch_icons();
    }

    pub(crate) fn relook(&mut self, key: &str, value: String) {
        let text_size = self.look.text_size;
        if !self.look.set(key, value) {
            return;
        }
        let theme = self.look.theme();
        self.ctx.text = text_for(&theme.fonts);
        publish_theme(&theme);
        self.ctx.theme = sized(theme, &*self.app, self.look.text_size);
        self.ctx.text_size = self.look.text_size;
        if key == reprisebus::appearance::ICON_THEME {
            let scale = self.ctx.icons.scale();
            self.ctx.icons = self.look.icons();
            self.ctx.icons.set_scale(scale);
            self.watch_icons();
        }
        for s in self.slots.iter_mut() {
            s.dirty = true;
        }
        self.menu_relooked();
        // Forms and the windows that hold them grow with the text size: each window hears it as
        // a resize and answers with its new size, then grows to what its program asks now.
        if self.look.text_size != text_size {
            let ids: Vec<WinId> = self.slots.iter().map(|s| s.id).collect();
            for &id in &ids {
                self.send(id, Event::Resized);
            }
            for id in ids {
                self.refit(id);
            }
        }
    }
}

/// How long the icon directories stay quiet before a change reloads the theme: one package
/// install is one reload.
const ICONS_SETTLE: Duration = Duration::from_millis(500);

#[cfg(test)]
mod tests;
