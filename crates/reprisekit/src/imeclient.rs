//! The focused edit as an input method's client for every host: what to tell the input method as the edit changes (`ImeClient::sync`, fed `Canvas::take_spot` after each frame) and what its preedit and commits do to the edit (`ImeClient::done`, returning editing keys). A program speaks text-input-v3, the compositor input-method-v2; both map the [`Request`]s onto their protocol.

use crate::raster::Rect;
use crate::textedit::{EditKey, Preedit, Surrounding, TextSpot};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    Enable,
    Disable,
    Surrounding(Surrounding),
    /// The surrounding text just sent changed by typing or a click, not through the input method.
    CausedElsewhere,
    ContentType {
        secret: bool,
        multiline: bool,
    },
    Cursor(Rect),
    Commit,
}

/// The requests that take the input method from `sent` (what it was last committed, `None` while disabled) to `want` (what the frame's focused edit reported). Another field is disabled and enabled afresh in two commits, so the input method drops what it was composing. `from_ime`: the text changed through the input method since.
pub fn plan(sent: Option<&TextSpot>, want: Option<&TextSpot>, from_ime: bool) -> Vec<Request> {
    let Some(want) = want else {
        return match sent {
            Some(_) => vec![Request::Disable, Request::Commit],
            None => Vec::new(),
        };
    };
    let same_field = sent
        .filter(|s| (s.field, s.secret, s.multiline) == (want.field, want.secret, want.multiline));
    let mut out = Vec::new();
    match same_field {
        Some(s) => {
            if s.surrounding != want.surrounding
                && let Some(sur) = &want.surrounding
            {
                out.push(Request::Surrounding(sur.clone()));
                if !from_ime {
                    out.push(Request::CausedElsewhere);
                }
            }
            if s.caret != want.caret {
                out.push(Request::Cursor(want.caret));
            }
        }
        None => {
            if sent.is_some() {
                out.extend([Request::Disable, Request::Commit]);
            }
            out.push(Request::Enable);
            if let Some(sur) = &want.surrounding {
                out.push(Request::Surrounding(sur.clone()));
            }
            out.push(Request::ContentType {
                secret: want.secret,
                multiline: want.multiline,
            });
            out.push(Request::Cursor(want.caret));
        }
    }
    if !out.is_empty() {
        out.push(Request::Commit);
    }
    out
}

/// The keys a `done` applies, in text-input-v3's order: the delete, then the commit. A preedit starting over a selection (`selected`) with nothing `composing` removes the selection first, which an empty insert does.
fn done_keys(p: &Pending, composing: bool, selected: bool) -> Vec<EditKey> {
    let mut keys = Vec::new();
    if p.preedit.is_some() && !composing && selected {
        keys.push(EditKey::Insert(String::new()));
    }
    if p.delete != (0, 0) {
        keys.push(EditKey::DeleteSurrounding {
            before: p.delete.0,
            after: p.delete.1,
        });
    }
    if let Some(text) = p.commit.as_ref().filter(|t| !t.is_empty()) {
        keys.push(EditKey::Insert(text.clone()));
    }
    keys
}

#[derive(Default)]
struct Pending {
    preedit: Option<Preedit>,
    commit: Option<String>,
    delete: (usize, usize),
}

#[derive(Default)]
pub struct ImeClient {
    sent: Option<TextSpot>,
    pending: Pending,
    pub preedit: Option<Preedit>,
    from_ime: bool,
}

impl ImeClient {
    pub fn sync(&mut self, spot: Option<TextSpot>) -> Vec<Request> {
        let spot = spot.filter(|s| !s.digits);
        let out = plan(self.sent.as_ref(), spot.as_ref(), self.from_ime);
        self.sent = spot;
        self.from_ime = false;
        out
    }

    pub fn sent(&self) -> Option<&TextSpot> {
        self.sent.as_ref()
    }

    /// Forget everything sent and pending: the input method was entered or left, so the next sync enables afresh. Returns whether a preedit was showing, which the edit now takes down.
    pub fn reset(&mut self) -> bool {
        self.sent = None;
        self.pending = Pending::default();
        self.from_ime = false;
        self.preedit.take().is_some()
    }

    pub fn preedit_string(&mut self, text: Option<String>, begin: i32, end: i32) {
        self.pending.preedit = Preedit::new(text.unwrap_or_default(), begin, end);
    }

    pub fn commit_string(&mut self, text: Option<String>) {
        self.pending.commit = text;
    }

    pub fn delete_surrounding(&mut self, before: u32, after: u32) {
        self.pending.delete = (before as usize, after as usize);
    }

    pub fn done(&mut self) -> Vec<EditKey> {
        let p = std::mem::take(&mut self.pending);
        let selected = self
            .sent
            .as_ref()
            .and_then(|s| s.surrounding.as_ref())
            .is_some_and(|s| s.cursor != s.anchor);
        let keys = done_keys(&p, self.preedit.is_some(), selected);
        self.preedit = p.preedit;
        self.from_ime |= !keys.is_empty();
        keys
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spot(field_x: i32, caret_x: i32, text: &str, cursor: usize) -> TextSpot {
        TextSpot {
            field: Rect::new(field_x, 10, 100, 15),
            caret: Rect::new(caret_x, 11, 1, 13),
            surrounding: Some(Surrounding {
                text: text.into(),
                cursor,
                anchor: cursor,
            }),
            secret: false,
            multiline: false,
            can_undo: false,
            selected: false,
            digits: false,
        }
    }

    #[test]
    fn a_focused_field_enables_with_its_state_and_losing_it_disables() {
        let a = spot(0, 5, "ab", 1);
        assert_eq!(
            plan(None, Some(&a), false),
            [
                Request::Enable,
                Request::Surrounding(a.surrounding.clone().unwrap()),
                Request::ContentType {
                    secret: false,
                    multiline: false
                },
                Request::Cursor(a.caret),
                Request::Commit,
            ]
        );
        assert_eq!(plan(Some(&a), Some(&a), false), [], "a blink sends nothing");
        assert_eq!(
            plan(Some(&a), None, false),
            [Request::Disable, Request::Commit]
        );
        assert_eq!(plan(None, None, false), []);
    }

    #[test]
    fn a_digits_field_turns_no_input_method_on() {
        let digits = TextSpot {
            digits: true,
            ..spot(0, 5, "10", 2)
        };
        let mut c = ImeClient::default();
        assert_eq!(c.sync(Some(digits.clone())), []);
        assert_eq!(c.sent(), None);
        c.sync(Some(spot(0, 5, "ab", 1)));
        assert_eq!(
            c.sync(Some(digits)),
            [Request::Disable, Request::Commit],
            "focus moving from a text field to a spin box turns it off"
        );
    }

    #[test]
    fn typing_updates_the_text_and_says_who_changed_it() {
        let a = spot(0, 5, "ab", 1);
        let b = spot(0, 9, "abc", 2);
        let sur = b.surrounding.clone().unwrap();
        assert_eq!(
            plan(Some(&a), Some(&b), false),
            [
                Request::Surrounding(sur.clone()),
                Request::CausedElsewhere,
                Request::Cursor(b.caret),
                Request::Commit,
            ]
        );
        assert_eq!(
            plan(Some(&a), Some(&b), true),
            [
                Request::Surrounding(sur),
                Request::Cursor(b.caret),
                Request::Commit,
            ],
            "a commit's own change is the input method's"
        );
    }

    #[test]
    fn another_field_is_disabled_and_enabled_in_two_commits() {
        let a = spot(0, 5, "ab", 1);
        let b = spot(200, 205, "", 0);
        let p = plan(Some(&a), Some(&b), false);
        assert_eq!(p[..3], [Request::Disable, Request::Commit, Request::Enable]);
        assert_eq!(p.last(), Some(&Request::Commit));
    }

    #[test]
    fn done_deletes_then_commits_and_a_new_preedit_takes_the_selection() {
        let p = Pending {
            preedit: Preedit::new("ni".into(), 2, 2),
            commit: Some("你好".into()),
            delete: (1, 0),
        };
        assert_eq!(
            done_keys(&p, false, true),
            [
                EditKey::Insert(String::new()),
                EditKey::DeleteSurrounding {
                    before: 1,
                    after: 0
                },
                EditKey::Insert("你好".into()),
            ]
        );
        assert_eq!(done_keys(&p, true, true).len(), 2, "already composing");
        assert_eq!(done_keys(&Pending::default(), false, true), []);
    }

    #[test]
    fn a_password_field_sends_its_purpose_and_no_text() {
        let pw = TextSpot {
            surrounding: None,
            secret: true,
            ..spot(0, 5, "", 0)
        };
        assert_eq!(
            plan(None, Some(&pw), false),
            [
                Request::Enable,
                Request::ContentType {
                    secret: true,
                    multiline: false
                },
                Request::Cursor(pw.caret),
                Request::Commit,
            ]
        );
    }

    #[test]
    fn a_commit_s_own_change_is_not_reported_as_typing_and_a_reset_enables_afresh() {
        let mut c = ImeClient::default();
        let a = spot(0, 5, "", 0);
        assert_eq!(c.sync(Some(a.clone()))[0], Request::Enable);
        c.preedit_string(Some("ni".into()), 2, 2);
        assert_eq!(c.done(), [], "a preedit alone types nothing");
        assert_eq!(c.preedit.as_ref().map(|p| p.text.as_str()), Some("ni"));
        c.commit_string(Some("你".into()));
        c.preedit_string(None, 0, 0);
        assert_eq!(c.done(), [EditKey::Insert("你".into())]);
        assert_eq!(c.preedit, None, "the commit ends the preedit");
        let b = spot(0, 17, "你", 3);
        assert!(
            !c.sync(Some(b.clone())).contains(&Request::CausedElsewhere),
            "the input method made this change"
        );
        assert!(!c.reset(), "no preedit was showing");
        assert_eq!(c.sync(Some(b))[0], Request::Enable);
    }
}
