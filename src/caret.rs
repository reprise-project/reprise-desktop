//! The compositor's own edit fields: which one has the keyboard, and its
//! caret. One timer toggles the caret every 530 ms; any edit or click
//! restarts it with the caret shown.

use std::time::Duration;

use smithay::reexports::calloop::{
    RegistrationToken,
    timer::{TimeoutAction, Timer},
};

use crate::compositor::State;

pub const BLINK: Duration = Duration::from_millis(530);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Editor {
    Launcher,
    StartPanel,
    Dialog,
    Rename,
    ClipView,
    EmojiView,
    NotifyReply,
}

pub struct Caret {
    timer: Option<RegistrationToken>,
    pub on: bool,
}

impl Default for Caret {
    fn default() -> Self {
        Caret {
            timer: None,
            on: true,
        }
    }
}

impl State {
    /// The field with the keyboard, in priority order: launcher, Start menu
    /// search, clipboard history, emoji picker, notification reply, top
    /// dialog, rename. Keys, the clipboard and the caret go to it.
    pub fn editor(&self) -> Option<Editor> {
        if self.launcher.is_some() {
            return Some(Editor::Launcher);
        }
        if self.start_panel.is_some() {
            return Some(Editor::StartPanel);
        }
        if self.panels.clipview.is_some() {
            return Some(Editor::ClipView);
        }
        if self.panels.emojiview.is_some() {
            return Some(Editor::EmojiView);
        }
        if crate::notifyview::reply_focused(self) {
            return Some(Editor::NotifyReply);
        }
        if self.dialog_has_keyboard()
            && self
                .dialogs
                .views
                .last()
                .is_some_and(|v| v.dialog.focused_edit().is_some())
        {
            return Some(Editor::Dialog);
        }
        self.desktop
            .as_ref()
            .is_some_and(|d| d.rename.is_some())
            .then_some(Editor::Rename)
    }

    pub fn restart_caret(&mut self) {
        self.caret.on = true;
        if let Some(t) = self.caret.timer.take() {
            self.handle.remove(t);
        }
        let token = self
            .handle
            .insert_source(Timer::from_duration(BLINK), |_, _, state| state.blink());
        match token {
            Ok(t) => self.caret.timer = Some(t),
            Err(e) => log::warn!("caret timer: {}", e.error),
        }
    }

    pub fn stop_caret(&mut self) {
        self.caret.on = true;
        if let Some(t) = self.caret.timer.take() {
            self.handle.remove(t);
        }
    }

    fn blink(&mut self) -> TimeoutAction {
        let Some(editor) = self.editor() else {
            self.caret.on = true;
            self.caret.timer = None;
            return TimeoutAction::Drop;
        };
        self.caret.on = !self.caret.on;
        match editor {
            Editor::Launcher => self.redraw_launcher(),
            Editor::Dialog => self.blink_dialog(),
            Editor::Rename => self.relayout_desktop(),
            Editor::StartPanel => self.redraw_start_panel(),
            Editor::ClipView => self.redraw_clipview(),
            Editor::EmojiView => self.redraw_emoji(),
            Editor::NotifyReply => crate::notifyview::blink(self),
        }
        TimeoutAction::ToDuration(BLINK)
    }
}
