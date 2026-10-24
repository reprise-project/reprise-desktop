//! Keys and clicks on the tab strip or the page list, and turning pages.

use crate::controls::form::{self, Form};
use crate::controls::pagelist;
use crate::controls::tabstrip::TabStrip;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StripKey {
    Pass,
    None,
    Moved,
    Turned,
}

/// The strip's part of a sheet's keys, for any host of a [`TabStrip`] over one
/// [`Form`] per tab. The strip is a tab stop between the page's last control
/// (the buttons come last) and its first:
///
/// - Tab off the page's last tab stop, or Shift+Tab off its first, puts the focus
///   on the strip; the page holds none meanwhile.
/// - Tab on the strip goes to the page's first tab stop, Shift+Tab to its last.
/// - Left and Right open the neighbouring tab, stopping at the ends; the strip
///   keeps the focus. Up and Down do nothing.
/// - `ctrl_tab` (`Some(back)`) opens the next or previous tab wherever the focus is.
///
/// A tab opened with the focus on the strip keeps it there; one opened from the
/// page focuses the new page's first tab stop. Everything else, Enter and Escape
/// included, is [`StripKey::Pass`]: the page's default button or Cancel answers.
pub fn strip_key(
    tabs: &mut TabStrip,
    pages: &mut [Form],
    key: &form::Key,
    ctrl_tab: Option<bool>,
) -> StripKey {
    nav_key(tabs, pages, key, ctrl_tab, false)
}

/// [`strip_key`] for either way of picking the page. With `list` the focused
/// list takes Up, Down, Home and End ([`pagelist::target`]); Left and Right do
/// nothing; Page Up and Down go to the page.
pub(super) fn nav_key(
    tabs: &mut TabStrip,
    pages: &mut [Form],
    key: &form::Key,
    ctrl_tab: Option<bool>,
    list: bool,
) -> StripKey {
    let was = tabs.open;
    if let Some(back) = ctrl_tab {
        return if tabs.cycle(back) {
            turned(tabs, pages, was);
            StripKey::Turned
        } else {
            StripKey::None
        };
    }
    let Some(page) = pages.get_mut(was) else {
        return StripKey::Pass;
    };
    // Hosts fold the arrows into editing keys for a focused field; on the
    // strip they are its own.
    let flat = match key {
        form::Key::Edit(k) if tabs.focused => form::flat_key(k),
        k => Some(k.clone()),
    };
    let Some(key) = flat else {
        return StripKey::Pass;
    };
    match key {
        form::Key::Tab { back } if tabs.focused => {
            if page.focus_edge(back) {
                tabs.focused = false;
                StripKey::Moved
            } else {
                StripKey::None
            }
        }
        // An open list takes Tab to close itself.
        form::Key::Tab { .. } if page.popup_open() => StripKey::Pass,
        form::Key::Tab { back } => {
            if !page.tab_no_wrap(back) {
                page.set_focus(None);
                tabs.focused = true;
            }
            StripKey::Moved
        }
        form::Key::Up | form::Key::Down | form::Key::Home | form::Key::End
            if tabs.focused && list =>
        {
            let to = pagelist::target(was, tabs.labels.len(), &key).unwrap_or(was);
            if to == was {
                return StripKey::None;
            }
            tabs.open = to;
            turned(tabs, pages, was);
            StripKey::Turned
        }
        form::Key::Left | form::Key::Right if tabs.focused && !list => {
            if tabs.step(key == form::Key::Right) {
                turned(tabs, pages, was);
                StripKey::Turned
            } else {
                StripKey::None
            }
        }
        form::Key::Left | form::Key::Right | form::Key::Up | form::Key::Down if tabs.focused => {
            StripKey::None
        }
        _ => StripKey::Pass,
    }
}

/// A press on tab `i`: a closed tab opens (see [`strip_key`]); the open one takes
/// the focus onto the strip.
pub fn tab_click(tabs: &mut TabStrip, pages: &mut [Form], i: usize) -> StripKey {
    nav_click(tabs, pages, i, false)
}

/// [`tab_click`] for either way of picking the page. With `list`, a press on a
/// closed page's row also focuses the list.
pub(super) fn nav_click(tabs: &mut TabStrip, pages: &mut [Form], i: usize, list: bool) -> StripKey {
    if i >= tabs.labels.len() {
        return StripKey::None;
    }
    if i != tabs.open {
        let was = tabs.open;
        tabs.open = i;
        tabs.focused |= list;
        turned(tabs, pages, was);
        return StripKey::Turned;
    }
    if tabs.focused {
        return StripKey::None;
    }
    tabs.focused = true;
    if let Some(page) = pages.get_mut(i) {
        page.set_focus(None);
    }
    StripKey::Moved
}

/// Whether a key press steps the sheet's pages, as `ctrl_tab` for
/// [`PropSheet::key`]: `Some(false)` for Ctrl+Tab and Ctrl+Page Down,
/// `Some(true)` for Ctrl+Shift+Tab and Ctrl+Page Up.
pub fn page_step(sym: u32, ctrl: bool, shift: bool) -> Option<bool> {
    use crate::textedit::keysym;
    if !ctrl {
        return None;
    }
    match sym {
        keysym::TAB => Some(shift),
        // Shift+Tab arrives as its own keysym.
        keysym::ISO_LEFT_TAB => Some(true),
        keysym::PAGE_DOWN | keysym::KP_PAGE_DOWN => Some(false),
        keysym::PAGE_UP | keysym::KP_PAGE_UP => Some(true),
        _ => None,
    }
}

/// After a press on the open `page`: a control that took the focus takes it from
/// the strip.
pub fn page_clicked(tabs: &mut TabStrip, page: &Form) {
    if page.focus().is_some() {
        tabs.focused = false;
    }
}

/// The open tab changed from `was`: the page left behind releases the pointer,
/// the new tab is no longer lit, and the new page's first tab stop takes the
/// focus unless the strip holds it.
pub(super) fn turned(tabs: &mut TabStrip, pages: &mut [Form], was: usize) {
    if let Some(old) = pages.get_mut(was) {
        old.cancel_pointer();
    }
    tabs.hot = None;
    if let Some(page) = pages.get_mut(tabs.open) {
        if tabs.focused {
            page.set_focus(None);
        } else {
            page.focus_edge(false);
        }
    }
}
