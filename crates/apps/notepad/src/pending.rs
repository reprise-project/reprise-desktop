//! The save prompt's bookkeeping: one action waits at a time and runs exactly once, after the document is saved
//! or discarded; any Cancel on the way drops it.

use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PendingAction {
    Quit,
    New,
    Open,
    OpenFile(PathBuf),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pending(Option<PendingAction>);

impl Pending {
    /// `action` was asked for. On a clean document it is handed straight
    /// back to run; on a dirty one it waits and the caller opens the
    /// prompt.
    pub fn ask(&mut self, action: PendingAction, dirty: bool) -> Option<PendingAction> {
        if dirty {
            self.0 = Some(action);
            None
        } else {
            Some(action)
        }
    }

    /// The prompt's Yes ran a save that `ok` says succeeded or not. The
    /// action runs on success; a failed save has its error box up, so
    /// the action is dropped rather than looking like Exit did nothing.
    pub fn saved(&mut self, ok: bool) -> Option<PendingAction> {
        self.0.take().filter(|_| ok)
    }

    pub fn discarded(&mut self) -> Option<PendingAction> {
        self.0.take()
    }

    /// Cancel anywhere on the way: the action is off and the document
    /// stays as it was.
    pub fn cancelled(&mut self) {
        self.0 = None;
    }

    /// Whether an action is still waiting. Yes on an Untitled document
    /// opens Save As without touching this: the dialog's own Accept or
    /// Cancel is what settles it.
    #[cfg(test)]
    pub fn is_waiting(&self) -> bool {
        self.0.is_some()
    }

    #[cfg(test)]
    pub fn waiting(action: PendingAction) -> Pending {
        Pending(Some(action))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clean_document_runs_the_action_at_once() {
        let mut p = Pending::default();
        assert_eq!(p.ask(PendingAction::Quit, false), Some(PendingAction::Quit));
        assert!(!p.is_waiting());
    }

    #[test]
    fn a_dirty_document_holds_the_action_for_the_prompt() {
        let mut p = Pending::default();
        assert_eq!(p.ask(PendingAction::New, true), None);
        assert!(p.is_waiting());
    }

    #[test]
    fn a_save_that_worked_runs_it_and_one_that_failed_drops_it() {
        let mut p = Pending::waiting(PendingAction::Open);
        assert_eq!(p.saved(true), Some(PendingAction::Open));
        assert!(!p.is_waiting());
        let mut p = Pending::waiting(PendingAction::Open);
        assert_eq!(p.saved(false), None);
        assert!(!p.is_waiting(), "dropped, not left for a later save");
    }

    #[test]
    fn discarding_runs_it_and_cancelling_drops_it() {
        let mut p = Pending::waiting(PendingAction::Quit);
        assert_eq!(p.discarded(), Some(PendingAction::Quit));
        let mut p = Pending::waiting(PendingAction::Quit);
        p.cancelled();
        assert!(!p.is_waiting());
        assert_eq!(p.discarded(), None, "nothing runs twice");
    }
}
