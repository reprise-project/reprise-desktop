use std::time::Instant;

use repriseapp::heads::{HeadConfig, HeadRequests, HeadSet};

use crate::displaypage::Display;
use crate::monitordlg::MonitorDialog;
use crate::outputwrite::OutputKeys;

pub const VRR_REFUSED: &str = "This monitor did not accept variable refresh rate.";

pub const KEEP_SECONDS: u32 = 15;

/// What the host does for the flow, in order. A `Quit` comes last and
/// only when nothing before it failed; the host skips it when a `Save`
/// before it could not be written, so that error stays on screen too.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    Save(String, OutputKeys),
    SavePrimary(String),
    Fail(&'static str),
    Quit,
}

pub struct Question {
    pub dialog: MonitorDialog,
    pub opened: Instant,
    asked: HeadSet,
    back: HeadSet,
}

#[derive(Default)]
pub struct Flow {
    /// The VRR requests out: (serial, output, value asked).
    vrr: Vec<(u32, String, bool)>,
    /// The mode and arrangement request out: (serial, what ran, what was asked).
    mode: Option<(u32, HeadSet, HeadSet)>,
    pub question: Option<Question>,
    /// Requests putting outputs back: (serial, what was asked, second try).
    reverts: Vec<(u32, HeadSet, bool)>,
    close: bool,
    discard: bool,
}

/// A mode, orientation or scale change writes all three as the output now
/// runs them, so no stale key stays beside the new one.
fn keys(back: HeadConfig, asked: HeadConfig) -> OutputKeys {
    let looks = asked.mode.is_some() || asked.transform.is_some() || asked.scale.is_some();
    let mut k = OutputKeys {
        position: asked.position,
        ..OutputKeys::default()
    };
    if looks {
        k.mode = asked
            .mode
            .or(back.mode)
            .map(|m| (m.width, m.height, m.refresh_mhz));
        k.transform = asked.transform.or(back.transform);
        k.scale = asked.scale.or(back.scale);
    }
    k
}

impl Flow {
    pub fn busy(&self) -> bool {
        !self.vrr.is_empty()
            || self.mode.is_some()
            || self.question.is_some()
            || !self.reverts.is_empty()
    }

    /// While busy the picks are not asked for; they stay dirty, so Apply stays lit.
    pub fn apply(&mut self, d: &mut Display, req: &mut HeadRequests, close: bool) -> Vec<Effect> {
        self.close |= close;
        let mut out = Vec::new();
        if !self.busy()
            && let Some(ch) = d.changes()
        {
            if let Some(p) = ch.primary {
                d.primary_saved();
                out.push(Effect::SavePrimary(p));
            }
            for (head, on) in ch.vrr {
                let cfg = HeadConfig {
                    adaptive_sync: Some(on),
                    ..HeadConfig::default()
                };
                self.vrr.push((req.configure(&head, cfg), head, on));
            }
            if !ch.set.is_empty() {
                let serial = req.configure_all(ch.set.clone());
                self.mode = Some((serial, ch.back, ch.set));
            }
        }
        out.extend(self.settle());
        out
    }

    pub fn cancel(&mut self, req: &mut HeadRequests) -> Vec<Effect> {
        self.close = true;
        self.discard = true;
        if self.question.is_some() {
            return self.answer(false, req);
        }
        self.settle()
    }

    fn revert(&mut self, back: HeadSet, req: &mut HeadRequests, retried: bool) {
        self.reverts
            .push((req.configure_all(back.clone()), back, retried));
    }

    /// While discarding, a change that took is asked back and one that failed
    /// says nothing.
    pub fn applied(
        &mut self,
        serial: u32,
        ok: bool,
        d: &mut Display,
        req: &mut HeadRequests,
    ) -> Vec<Effect> {
        let mut out = Vec::new();
        if let Some(i) = self.vrr.iter().position(|v| v.0 == serial) {
            let (_, head, on) = self.vrr.remove(i);
            if ok && self.discard {
                let cfg = HeadConfig {
                    adaptive_sync: Some(!on),
                    ..HeadConfig::default()
                };
                self.revert(vec![(head, cfg)], req, false);
            } else if ok {
                let keys = OutputKeys {
                    adaptive_sync: Some(on),
                    ..OutputKeys::default()
                };
                out.push(Effect::Save(head, keys));
            } else {
                d.reset_vrr(&head);
                self.close &= self.discard;
                if !self.discard {
                    out.push(Effect::Fail(VRR_REFUSED));
                }
            }
        } else if let Some((_, back, asked)) = self.mode.take_if(|m| m.0 == serial) {
            if ok && self.discard {
                self.revert(back, req, false);
            } else if ok {
                self.question = Some(Question {
                    dialog: MonitorDialog::new(KEEP_SECONDS, None),
                    opened: Instant::now(),
                    asked,
                    back,
                });
            } else if !self.discard {
                self.close = false;
                out.push(Effect::Fail("The display settings could not be applied."));
            }
        } else if let Some(i) = self.reverts.iter().position(|r| r.0 == serial) {
            let (_, set, retried) = self.reverts.remove(i);
            let listed = set.iter().any(|(n, _)| d.head_named(n).is_some());
            if !ok && listed && !retried {
                self.revert(set, req, true);
            } else if !ok && listed {
                self.close = false;
                self.discard = false;
                out.push(Effect::Fail(
                    "The old display settings could not be put back.",
                ));
            }
        }
        out.extend(self.settle());
        out
    }

    pub fn answer(&mut self, keep: bool, req: &mut HeadRequests) -> Vec<Effect> {
        let Some(q) = self.question.take() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        if keep {
            for (name, asked) in q.asked {
                let back = q
                    .back
                    .iter()
                    .find(|(n, _)| *n == name)
                    .map_or_else(HeadConfig::default, |b| b.1);
                out.push(Effect::Save(name, keys(back, asked)));
            }
        } else {
            self.revert(q.back, req, false);
        }
        out.extend(self.settle());
        out
    }

    pub fn tick(&mut self, now: Instant, req: &mut HeadRequests) -> Vec<Effect> {
        let Some(q) = self.question.as_mut() else {
            return Vec::new();
        };
        let gone = now.saturating_duration_since(q.opened).as_secs();
        let left = u64::from(KEEP_SECONDS).saturating_sub(gone) as u32;
        q.dialog.set_left(left);
        if left == 0 {
            return self.answer(false, req);
        }
        Vec::new()
    }

    /// The sheet's close box, which is Cancel. The question's own close box is
    /// No ([`Flow::answer`]) and leaves the sheet open.
    pub fn close(&mut self, req: &mut HeadRequests) -> Vec<Effect> {
        self.cancel(req)
    }

    pub fn heads_changed(&mut self, d: &Display) -> Vec<Effect> {
        if self
            .question
            .as_ref()
            .is_some_and(|q| q.asked.iter().any(|(n, _)| d.head_named(n).is_none()))
        {
            self.question = None;
        }
        self.settle()
    }

    fn settle(&mut self) -> Vec<Effect> {
        if self.close && !self.busy() {
            self.close = false;
            self.discard = false;
            return vec![Effect::Quit];
        }
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::displaypage::test_head;
    use repriseapp::heads::{HeadMode, Transform};
    use std::time::Duration;

    fn take(req: &mut HeadRequests) -> Vec<(u32, String, HeadConfig)> {
        req.take()
            .into_iter()
            .map(|(serial, set)| {
                assert_eq!(set.len(), 1, "one output: {set:?}");
                let (name, cfg) = set[0].clone();
                (serial, name, cfg)
            })
            .collect()
    }

    fn smaller() -> (Display, HeadRequests) {
        let mut d = Display::with_heads(vec![test_head("DP-1")]);
        d.pick_resolution(1);
        (d, HeadRequests::default())
    }

    const HD: HeadMode = HeadMode {
        width: 1280,
        height: 720,
        refresh_mhz: 60_000,
        preferred: false,
    };

    #[test]
    fn yes_writes_mode_orientation_and_scale() {
        let (mut d, mut req) = smaller();
        let mut flow = Flow::default();
        assert_eq!(
            flow.apply(&mut d, &mut req, false),
            vec![],
            "nothing written yet"
        );
        let asked = take(&mut req);
        assert_eq!(asked.len(), 1);
        let (serial, name, cfg) = asked[0].clone();
        assert_eq!((name.as_str(), cfg.mode), ("DP-1", Some(HD)));
        assert_eq!(flow.applied(serial, true, &mut d, &mut req), vec![]);
        assert!(flow.question.is_some(), "the question is up");
        assert_eq!(
            flow.answer(true, &mut req),
            vec![Effect::Save(
                "DP-1".into(),
                OutputKeys {
                    mode: Some((1280, 720, 60_000)),
                    transform: Some(Transform::Normal),
                    scale: Some(1.0),
                    ..OutputKeys::default()
                }
            )],
            "all three, so no stale key stays beside the new mode"
        );
        assert!(flow.question.is_none() && take(&mut req).is_empty());
    }

    #[test]
    fn no_and_the_timeout_put_the_old_mode_back_and_write_nothing() {
        for timeout in [false, true] {
            let (mut d, mut req) = smaller();
            let mut flow = Flow::default();
            flow.apply(&mut d, &mut req, false);
            let serial = take(&mut req)[0].0;
            flow.applied(serial, true, &mut d, &mut req);
            let opened = flow.question.as_ref().unwrap().opened;
            let out = if timeout {
                assert_eq!(flow.tick(opened + Duration::from_secs(5), &mut req), vec![]);
                assert_eq!(flow.question.as_ref().unwrap().dialog.left, 10);
                flow.tick(opened + Duration::from_secs(15), &mut req)
            } else {
                flow.answer(false, &mut req)
            };
            assert_eq!(out, vec![], "nothing written");
            let back = take(&mut req);
            assert_eq!(back.len(), 1);
            assert_eq!(
                back[0].2,
                test_head("DP-1").state(),
                "the old mode, orientation and scale"
            );
            assert!(flow.question.is_none());
        }
    }

    #[test]
    fn ok_closes_only_after_the_answer() {
        let (mut d, mut req) = smaller();
        let mut flow = Flow::default();
        assert_eq!(flow.apply(&mut d, &mut req, true), vec![]);
        let serial = take(&mut req)[0].0;
        assert_eq!(
            flow.applied(serial, true, &mut d, &mut req),
            vec![],
            "the question first"
        );
        let out = flow.answer(true, &mut req);
        assert_eq!(out.last(), Some(&Effect::Quit));
        let (mut d, mut req) = smaller();
        let mut flow = Flow::default();
        flow.apply(&mut d, &mut req, true);
        let serial = take(&mut req)[0].0;
        flow.applied(serial, true, &mut d, &mut req);
        assert_eq!(
            flow.answer(false, &mut req),
            vec![],
            "the revert is still out"
        );
        let revert = take(&mut req)[0].0;
        assert_eq!(
            flow.applied(revert, true, &mut d, &mut req),
            vec![Effect::Quit]
        );
    }

    #[test]
    fn nothing_to_apply_quits_at_once_on_ok() {
        let mut d = Display::with_heads(vec![test_head("DP-1")]);
        let mut req = HeadRequests::default();
        let mut flow = Flow::default();
        assert_eq!(flow.apply(&mut d, &mut req, false), vec![]);
        assert_eq!(flow.apply(&mut d, &mut req, true), vec![Effect::Quit]);
        assert!(take(&mut req).is_empty());
    }

    #[test]
    fn a_failed_apply_says_so_and_keeps_the_sheet_and_the_picks() {
        let (mut d, mut req) = smaller();
        let mut flow = Flow::default();
        flow.apply(&mut d, &mut req, true);
        let serial = take(&mut req)[0].0;
        let out = flow.applied(serial, false, &mut d, &mut req);
        assert_eq!(
            out,
            vec![Effect::Fail("The display settings could not be applied.")]
        );
        assert!(flow.question.is_none() && !flow.busy());
        assert!(d.dirty(), "the picks stay, Apply stays lit");
    }

    #[test]
    fn vrr_is_saved_when_taken_and_unticked_with_an_error_when_refused() {
        for ok in [true, false] {
            let mut d = Display::with_heads(vec![test_head("DP-1")]);
            let mut req = HeadRequests::default();
            let mut flow = Flow::default();
            d.set_vrr(true);
            flow.apply(&mut d, &mut req, false);
            let asked = take(&mut req);
            assert_eq!(asked[0].2.adaptive_sync, Some(true));
            let out = flow.applied(asked[0].0, ok, &mut d, &mut req);
            if ok {
                let keys = OutputKeys {
                    adaptive_sync: Some(true),
                    ..OutputKeys::default()
                };
                assert_eq!(out, vec![Effect::Save("DP-1".into(), keys)]);
            } else {
                assert_eq!(
                    out,
                    vec![Effect::Fail(VRR_REFUSED)],
                    "a refusal writes nothing and says so"
                );
                assert!(!d.pick().unwrap().vrr, "and unticks");
            }
        }
    }

    #[test]
    fn vrr_and_mode_go_as_two_requests_vrr_first() {
        let (mut d, mut req) = smaller();
        d.set_vrr(true);
        let mut flow = Flow::default();
        flow.apply(&mut d, &mut req, true);
        let asked = take(&mut req);
        assert_eq!(asked.len(), 2);
        assert_eq!(
            (asked[0].2.adaptive_sync, asked[0].2.mode),
            (Some(true), None)
        );
        assert_eq!(
            (asked[1].2.adaptive_sync, asked[1].2.mode),
            (None, Some(HD))
        );
        assert_eq!(
            flow.applied(asked[0].0, false, &mut d, &mut req),
            vec![Effect::Fail(VRR_REFUSED)]
        );
        flow.applied(asked[1].0, true, &mut d, &mut req);
        let out = flow.answer(true, &mut req);
        assert!(
            !out.contains(&Effect::Quit),
            "a refused VRR keeps the sheet open: {out:?}"
        );
    }

    #[test]
    fn the_close_box_goes_back_first() {
        let (mut d, mut req) = smaller();
        let mut flow = Flow::default();
        flow.apply(&mut d, &mut req, false);
        let serial = take(&mut req)[0].0;
        flow.applied(serial, true, &mut d, &mut req);
        assert_eq!(flow.close(&mut req), vec![]);
        let revert = take(&mut req);
        assert_eq!(revert.len(), 1, "the old mode is asked back");
        assert_eq!(
            flow.applied(revert[0].0, true, &mut d, &mut req),
            vec![Effect::Quit]
        );
    }

    #[test]
    fn a_question_whose_output_went_away_closes() {
        let (mut d, mut req) = smaller();
        let mut flow = Flow::default();
        flow.apply(&mut d, &mut req, false);
        let serial = take(&mut req)[0].0;
        flow.applied(serial, true, &mut d, &mut req);
        d.set_heads(vec![test_head("HDMI-A-1")]);
        assert_eq!(flow.heads_changed(&d), vec![]);
        assert!(flow.question.is_none() && take(&mut req).is_empty());
    }

    fn cancelled(
        vrr: bool,
        mode: bool,
    ) -> (Display, HeadRequests, Flow, Vec<(u32, String, HeadConfig)>) {
        let mut d = Display::with_heads(vec![test_head("DP-1")]);
        if mode {
            d.pick_resolution(1);
        }
        d.set_vrr(vrr);
        let mut req = HeadRequests::default();
        let mut flow = Flow::default();
        flow.apply(&mut d, &mut req, false);
        let asked = take(&mut req);
        assert_eq!(flow.cancel(&mut req), vec![], "waits for the compositor");
        (d, req, flow, asked)
    }

    #[test]
    fn a_mode_that_lands_after_cancel_goes_straight_back() {
        let (mut d, mut req, mut flow, asked) = cancelled(false, true);
        assert_eq!(flow.applied(asked[0].0, true, &mut d, &mut req), vec![]);
        assert!(flow.question.is_none(), "no question");
        let back = take(&mut req);
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].2, test_head("DP-1").state());
        assert_eq!(
            flow.applied(back[0].0, true, &mut d, &mut req),
            vec![Effect::Quit]
        );
    }

    #[test]
    fn a_vrr_that_lands_after_cancel_goes_back_unsaved() {
        let (mut d, mut req, mut flow, asked) = cancelled(true, false);
        assert_eq!(
            flow.applied(asked[0].0, true, &mut d, &mut req),
            vec![],
            "nothing saved"
        );
        let back = take(&mut req);
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].2.adaptive_sync, Some(false));
        assert_eq!(
            flow.applied(back[0].0, true, &mut d, &mut req),
            vec![Effect::Quit]
        );
    }

    #[test]
    fn a_failure_after_cancel_quits_without_an_error() {
        for (vrr, mode) in [(false, true), (true, false)] {
            let (mut d, mut req, mut flow, asked) = cancelled(vrr, mode);
            assert_eq!(
                flow.applied(asked[0].0, false, &mut d, &mut req),
                vec![Effect::Quit],
                "vrr {vrr}, mode {mode}"
            );
            assert!(take(&mut req).is_empty());
        }
    }

    #[test]
    fn cancel_with_nothing_out_quits_at_once() {
        let mut req = HeadRequests::default();
        assert_eq!(Flow::default().cancel(&mut req), vec![Effect::Quit]);
        assert!(take(&mut req).is_empty());
    }

    #[test]
    fn a_failed_revert_is_tried_once_more_then_reported() {
        for second_ok in [true, false] {
            let (mut d, mut req) = smaller();
            let mut flow = Flow::default();
            flow.apply(&mut d, &mut req, true);
            let serial = take(&mut req)[0].0;
            flow.applied(serial, true, &mut d, &mut req);
            flow.answer(false, &mut req);
            let first = take(&mut req);
            assert_eq!(flow.applied(first[0].0, false, &mut d, &mut req), vec![]);
            let again = take(&mut req);
            assert_eq!(again.len(), 1, "the same revert goes out again");
            assert_eq!(again[0].2, first[0].2);
            assert!(flow.busy());
            let out = flow.applied(again[0].0, second_ok, &mut d, &mut req);
            if second_ok {
                assert_eq!(out, vec![Effect::Quit], "no error");
            } else {
                assert_eq!(
                    out,
                    vec![Effect::Fail(
                        "The old display settings could not be put back."
                    )]
                );
                assert!(
                    take(&mut req).is_empty() && !flow.busy(),
                    "no Quit over the box"
                );
            }
        }
    }

    #[test]
    fn a_revert_for_a_vanished_output_fails_quietly() {
        let (mut d, mut req) = smaller();
        let mut flow = Flow::default();
        flow.apply(&mut d, &mut req, false);
        let serial = take(&mut req)[0].0;
        flow.applied(serial, true, &mut d, &mut req);
        flow.answer(false, &mut req);
        let revert = take(&mut req)[0].0;
        d.set_heads(vec![test_head("HDMI-A-1")]);
        assert_eq!(flow.applied(revert, false, &mut d, &mut req), vec![]);
        assert!(!flow.busy());
    }
}
