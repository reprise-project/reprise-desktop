use std::time::Instant;

use repriseapp::heads::{AdaptiveSync, HeadConfig, HeadRequests, HeadSet, TrialEnd};
use repriseconf::config::VrrPolicy;

use crate::displaypage::Display;
use crate::monitordlg::MonitorDialog;
use crate::outputwrite::OutputKeys;

pub const VRR_REFUSED: &str = "This monitor did not accept variable refresh rate.";

pub const KEEP_SECONDS: u32 = 15;

const NOT_PUT_BACK: &str = "The old display settings could not be put back.";

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
    /// When Settings asked for the trial: never later than the compositor's begin, so this
    /// countdown runs out first.
    pub opened: Instant,
    asked: HeadSet,
    back: HeadSet,
}

/// The mode and arrangement change's trial, which the compositor holds and puts back.
#[derive(Clone, Debug, Default, PartialEq)]
enum Trial {
    #[default]
    None,
    /// Asked for at this instant, neither kept nor reverted yet.
    Held(Instant),
    /// Revert sent; waiting for the trial to end.
    Reverting,
    /// Keep sent; these are written once the compositor says it kept the trial.
    Keeping(Vec<Effect>),
}

#[derive(Default)]
pub struct Flow {
    /// The VRR requests out: (serial, output, value asked, value it ran).
    vrr: Vec<(u32, String, bool, bool)>,
    /// The mode and arrangement request out: (serial, what ran, what was asked).
    mode: Option<(u32, HeadSet, HeadSet)>,
    pub question: Option<Question>,
    trial: Trial,
    /// Requests putting VRR back after Cancel: (serial, output).
    vrr_back: Vec<(u32, String)>,
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

/// The seconds a trial begun at `began` has left at `now`.
fn left(began: Instant, now: Instant) -> u32 {
    let gone = now.saturating_duration_since(began).as_secs();
    u64::from(KEEP_SECONDS).saturating_sub(gone) as u32
}

impl Flow {
    pub fn busy(&self) -> bool {
        !self.vrr.is_empty()
            || self.mode.is_some()
            || self.question.is_some()
            || matches!(self.trial, Trial::Reverting | Trial::Keeping(_))
            || !self.vrr_back.is_empty()
    }

    /// While busy the picks are not asked for; they stay dirty, so Apply stays lit.
    pub fn apply(
        &mut self,
        d: &mut Display,
        req: &mut HeadRequests,
        close: bool,
        now: Instant,
    ) -> Vec<Effect> {
        self.close |= close;
        let mut out = Vec::new();
        if !self.busy()
            && let Some(ch) = d.changes()
        {
            if let Some(p) = ch.primary {
                d.primary_saved();
                out.push(Effect::SavePrimary(p));
            }
            if !ch.profiles.is_empty() {
                d.profiles_written();
            }
            for (head, p) in ch.profiles {
                let keys = OutputKeys {
                    icc_profile: Some(p),
                    ..OutputKeys::default()
                };
                out.push(Effect::Save(head, keys));
            }
            for (head, p) in ch.vrr {
                // The compositor takes only on or off; "fullscreen" reaches it through the file.
                if p == VrrPolicy::Fullscreen {
                    d.vrr_written(&head, p);
                    let keys = OutputKeys {
                        adaptive_sync: Some(p),
                        ..OutputKeys::default()
                    };
                    out.push(Effect::Save(head, keys));
                    continue;
                }
                let on = p == VrrPolicy::On;
                let was = d
                    .head_named(&head)
                    .is_some_and(|h| h.adaptive_sync == AdaptiveSync::On);
                let cfg = HeadConfig {
                    adaptive_sync: Some(on),
                    ..HeadConfig::default()
                };
                self.vrr.push((req.configure(&head, cfg), head, on, was));
            }
            if !ch.set.is_empty() {
                let serial = req.configure_on_trial(ch.set.clone(), KEEP_SECONDS);
                self.mode = Some((serial, ch.back, ch.set));
                self.trial = Trial::Held(now);
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

    fn revert(&mut self, req: &mut HeadRequests) {
        req.revert_trial();
        self.trial = Trial::Reverting;
    }

    /// While discarding, a change that took is asked back and one that failed
    /// says nothing. A mode that failed has its trial reverted without waiting, so what part
    /// of it took goes back.
    pub fn applied(
        &mut self,
        serial: u32,
        ok: bool,
        d: &mut Display,
        req: &mut HeadRequests,
        now: Instant,
    ) -> Vec<Effect> {
        let mut out = Vec::new();
        if let Some(i) = self.vrr.iter().position(|v| v.0 == serial) {
            let (_, head, on, was) = self.vrr.remove(i);
            if ok && self.discard {
                let cfg = HeadConfig {
                    adaptive_sync: Some(was),
                    ..HeadConfig::default()
                };
                self.vrr_back.push((req.configure(&head, cfg), head));
            } else if ok {
                d.vrr_written(&head, VrrPolicy::of(on));
                let keys = OutputKeys {
                    adaptive_sync: Some(VrrPolicy::of(on)),
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
            let held = match self.trial {
                Trial::Held(began) => Some(began),
                _ => None,
            };
            if !ok && held.is_some() {
                req.revert_trial();
                self.trial = Trial::None;
            }
            match held {
                Some(_) if ok && self.discard => self.revert(req),
                // An apply that took the whole trial ends as No, without a box at 0.
                Some(began) if ok && left(began, now) == 0 => self.revert(req),
                Some(began) if ok => {
                    let mut q = Question {
                        dialog: MonitorDialog::new(KEEP_SECONDS, None),
                        opened: began,
                        asked,
                        back,
                    };
                    q.dialog.set_left(left(began, now));
                    self.question = Some(q);
                }
                _ if !ok && !self.discard => {
                    self.close = false;
                    out.push(Effect::Fail("The display settings could not be applied."));
                }
                _ => {}
            }
        } else if let Some(i) = self.vrr_back.iter().position(|v| v.0 == serial) {
            let (_, head) = self.vrr_back.remove(i);
            if !ok && d.head_named(&head).is_some() {
                self.close = false;
                self.discard = false;
                out.push(Effect::Fail(NOT_PUT_BACK));
            }
        }
        out.extend(self.settle());
        out
    }

    pub fn answer(&mut self, keep: bool, req: &mut HeadRequests) -> Vec<Effect> {
        let Some(q) = self.question.take() else {
            return Vec::new();
        };
        if keep {
            req.keep_trial();
            // A display turned off keeps its `[[outputs]]` keys for when it comes back.
            let saves = (q.asked.into_iter())
                .filter(|(_, a)| a.enabled != Some(false))
                .map(|(name, asked)| {
                    let back = (q.back.iter())
                        .find(|(n, _)| *n == name)
                        .map_or_else(HeadConfig::default, |b| b.1);
                    Effect::Save(name, keys(back, asked))
                })
                .collect();
            self.trial = Trial::Keeping(saves);
        } else {
            self.revert(req);
        }
        self.settle()
    }

    /// The trial is over. Kept after Yes: the keys are written. Otherwise the compositor put the
    /// outputs back (its seconds ran out, maybe before Yes reached it), which answers the
    /// question No; a put back that failed says so.
    pub fn trial_ended(&mut self, end: TrialEnd) -> Vec<Effect> {
        let mut out = Vec::new();
        match std::mem::take(&mut self.trial) {
            Trial::None => return out,
            Trial::Keeping(saves) if end == TrialEnd::Kept => out = saves,
            _ => {}
        }
        self.question = None;
        if end == TrialEnd::Failed {
            self.close = false;
            self.discard = false;
            out.push(Effect::Fail(NOT_PUT_BACK));
        }
        out.extend(self.settle());
        out
    }

    pub fn tick(&mut self, now: Instant, req: &mut HeadRequests) -> Vec<Effect> {
        let Some(q) = self.question.as_mut() else {
            return Vec::new();
        };
        let left = left(q.opened, now);
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

    /// An output the question asked about going away drops the question, and the trial puts the
    /// outputs still there back to what they ran before.
    pub fn heads_changed(&mut self, d: &Display, req: &mut HeadRequests) -> Vec<Effect> {
        if (self.question)
            .take_if(|q| q.asked.iter().any(|(n, _)| d.head_named(n).is_none()))
            .is_some()
        {
            self.revert(req);
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
    use repriseapp::heads::{HeadMode, Transform, TrialAsk, TrialEnd};
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
            flow.apply(&mut d, &mut req, false, Instant::now()),
            vec![],
            "nothing written yet"
        );
        let asked = take(&mut req);
        assert_eq!(asked.len(), 1);
        let (serial, name, cfg) = asked[0].clone();
        assert_eq!((name.as_str(), cfg.mode), ("DP-1", Some(HD)));
        assert_eq!(
            req.take_trial(),
            [TrialAsk::Begin {
                serial,
                seconds: KEEP_SECONDS
            }],
            "the mode goes on trial"
        );
        assert_eq!(
            flow.applied(serial, true, &mut d, &mut req, Instant::now()),
            vec![]
        );
        assert!(flow.question.is_some(), "the question is up");
        assert_eq!(
            flow.answer(true, &mut req),
            vec![],
            "nothing written before the compositor kept it"
        );
        assert!(flow.question.is_none() && flow.busy());
        assert_eq!(
            flow.trial_ended(TrialEnd::Kept),
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
        assert!(take(&mut req).is_empty());
        assert_eq!(req.take_trial(), [TrialAsk::Keep]);
        assert!(!flow.busy());
    }

    #[test]
    fn yes_to_a_trial_the_compositor_already_ended_writes_nothing() {
        for end in [TrialEnd::Reverted, TrialEnd::Failed] {
            let (mut d, mut req) = smaller();
            let mut flow = Flow::default();
            flow.apply(&mut d, &mut req, true, Instant::now());
            let serial = take(&mut req)[0].0;
            flow.applied(serial, true, &mut d, &mut req, Instant::now());
            assert_eq!(flow.answer(true, &mut req), vec![]);
            let fail = match end {
                TrialEnd::Failed => vec![Effect::Fail(NOT_PUT_BACK)],
                _ => vec![Effect::Quit],
            };
            assert_eq!(flow.trial_ended(end), fail, "as after No");
            assert!(!flow.busy());
        }
    }

    #[test]
    fn no_and_the_timeout_revert_the_trial_and_write_nothing() {
        for timeout in [false, true] {
            let (mut d, mut req) = smaller();
            let mut flow = Flow::default();
            flow.apply(&mut d, &mut req, false, Instant::now());
            let serial = take(&mut req)[0].0;
            flow.applied(serial, true, &mut d, &mut req, Instant::now());
            let opened = flow.question.as_ref().unwrap().opened;
            let out = if timeout {
                assert_eq!(flow.tick(opened + Duration::from_secs(5), &mut req), vec![]);
                assert_eq!(flow.question.as_ref().unwrap().dialog.left, 10);
                flow.tick(opened + Duration::from_secs(15), &mut req)
            } else {
                flow.answer(false, &mut req)
            };
            assert_eq!(out, vec![], "nothing written");
            assert!(take(&mut req).is_empty(), "the compositor puts it back");
            assert_eq!(req.take_trial().last(), Some(&TrialAsk::Revert));
            assert!(flow.question.is_none());
            assert!(flow.busy(), "until the trial has ended");
            assert_eq!(flow.trial_ended(TrialEnd::Reverted), vec![]);
            assert!(!flow.busy());
        }
    }

    #[test]
    fn the_countdown_counts_from_the_trials_begin() {
        let (mut d, mut req) = smaller();
        let mut flow = Flow::default();
        let began = Instant::now();
        flow.apply(&mut d, &mut req, false, began);
        let serial = take(&mut req)[0].0;
        flow.applied(
            serial,
            true,
            &mut d,
            &mut req,
            began + Duration::from_secs(4),
        );
        let q = flow.question.as_ref().unwrap();
        assert_eq!(q.opened, began);
        assert_eq!(
            q.dialog.left,
            KEEP_SECONDS - 4,
            "the seconds the trial has left"
        );
    }

    #[test]
    fn an_apply_that_took_the_whole_trial_ends_as_no_without_a_box() {
        let (mut d, mut req) = smaller();
        let mut flow = Flow::default();
        let began = Instant::now();
        flow.apply(&mut d, &mut req, false, began);
        let serial = take(&mut req)[0].0;
        let late = began + Duration::from_secs(KEEP_SECONDS.into());
        assert_eq!(flow.applied(serial, true, &mut d, &mut req, late), vec![]);
        assert!(flow.question.is_none(), "no box at 0");
        assert_eq!(req.take_trial().last(), Some(&TrialAsk::Revert));
        assert_eq!(flow.trial_ended(TrialEnd::Reverted), vec![]);
        assert!(!flow.busy());
    }

    #[test]
    fn a_trial_the_compositor_put_back_closes_the_question_as_no() {
        for ok in [false, true] {
            let (mut d, mut req) = smaller();
            let mut flow = Flow::default();
            flow.apply(&mut d, &mut req, ok, Instant::now());
            let serial = take(&mut req)[0].0;
            flow.applied(serial, true, &mut d, &mut req, Instant::now());
            req.take_trial();
            let out = flow.trial_ended(TrialEnd::Reverted);
            let quit = if ok { vec![Effect::Quit] } else { vec![] };
            assert_eq!(
                out, quit,
                "nothing written; OK closes the sheet as after No"
            );
            assert!(flow.question.is_none() && !flow.busy());
            assert!(req.take_trial().is_empty(), "nothing left to revert");
        }
    }

    /// Settings stalled before its configure went out: the trial ends first, then the
    /// configure, too late for it, fails.
    #[test]
    fn a_trial_that_ended_before_its_mode_was_applied_shows_the_failure() {
        let (mut d, mut req) = smaller();
        let mut flow = Flow::default();
        flow.apply(&mut d, &mut req, false, Instant::now());
        let serial = take(&mut req)[0].0;
        req.take_trial();
        assert_eq!(flow.trial_ended(TrialEnd::Reverted), vec![]);
        assert_eq!(
            flow.applied(serial, false, &mut d, &mut req, Instant::now()),
            vec![Effect::Fail("The display settings could not be applied.")]
        );
        assert!(flow.question.is_none() && !flow.busy());
    }

    #[test]
    fn ok_closes_only_after_the_answer() {
        let (mut d, mut req) = smaller();
        let mut flow = Flow::default();
        assert_eq!(flow.apply(&mut d, &mut req, true, Instant::now()), vec![]);
        let serial = take(&mut req)[0].0;
        assert_eq!(
            flow.applied(serial, true, &mut d, &mut req, Instant::now()),
            vec![],
            "the question first"
        );
        assert_eq!(flow.answer(true, &mut req), vec![], "the keep is still out");
        let out = flow.trial_ended(TrialEnd::Kept);
        assert!(matches!(out.first(), Some(Effect::Save(..))), "{out:?}");
        assert_eq!(out.last(), Some(&Effect::Quit));
        let (mut d, mut req) = smaller();
        let mut flow = Flow::default();
        flow.apply(&mut d, &mut req, true, Instant::now());
        let serial = take(&mut req)[0].0;
        flow.applied(serial, true, &mut d, &mut req, Instant::now());
        assert_eq!(
            flow.answer(false, &mut req),
            vec![],
            "the revert is still out"
        );
        assert_eq!(flow.trial_ended(TrialEnd::Reverted), vec![Effect::Quit]);
    }

    #[test]
    fn nothing_to_apply_quits_at_once_on_ok() {
        let mut d = Display::with_heads(vec![test_head("DP-1")]);
        let mut req = HeadRequests::default();
        let mut flow = Flow::default();
        assert_eq!(flow.apply(&mut d, &mut req, false, Instant::now()), vec![]);
        assert_eq!(
            flow.apply(&mut d, &mut req, true, Instant::now()),
            vec![Effect::Quit]
        );
        assert!(take(&mut req).is_empty());
    }

    #[test]
    fn a_failed_apply_says_so_and_keeps_the_sheet_and_the_picks() {
        let (mut d, mut req) = smaller();
        let mut flow = Flow::default();
        flow.apply(&mut d, &mut req, true, Instant::now());
        let serial = take(&mut req)[0].0;
        req.take_trial();
        let out = flow.applied(serial, false, &mut d, &mut req, Instant::now());
        assert_eq!(
            out,
            vec![Effect::Fail("The display settings could not be applied.")]
        );
        assert!(flow.question.is_none() && !flow.busy());
        assert!(d.dirty(), "the picks stay, Apply stays lit");
        assert_eq!(
            req.take_trial(),
            [TrialAsk::Revert],
            "what part of it took goes back, without waiting"
        );
        assert_eq!(
            flow.trial_ended(TrialEnd::Failed),
            vec![],
            "a trial no longer ours"
        );
    }

    #[test]
    fn vrr_is_saved_when_taken_and_unticked_with_an_error_when_refused() {
        for ok in [true, false] {
            let mut d = Display::with_heads(vec![test_head("DP-1")]);
            let mut req = HeadRequests::default();
            let mut flow = Flow::default();
            d.set_vrr(VrrPolicy::On);
            flow.apply(&mut d, &mut req, false, Instant::now());
            let asked = take(&mut req);
            assert_eq!(asked[0].2.adaptive_sync, Some(true));
            let out = flow.applied(asked[0].0, ok, &mut d, &mut req, Instant::now());
            if ok {
                let keys = OutputKeys {
                    adaptive_sync: Some(VrrPolicy::On),
                    ..OutputKeys::default()
                };
                assert_eq!(out, vec![Effect::Save("DP-1".into(), keys)]);
            } else {
                assert_eq!(
                    out,
                    vec![Effect::Fail(VRR_REFUSED)],
                    "a refusal writes nothing and says so"
                );
                assert_eq!(
                    d.pick().unwrap().vrr,
                    VrrPolicy::Off,
                    "and goes back to Off"
                );
            }
        }
    }

    #[test]
    fn fullscreen_is_saved_without_a_request_and_shows_as_picked() {
        let mut d = Display::with_heads(vec![test_head("DP-1")]);
        let mut req = HeadRequests::default();
        let mut flow = Flow::default();
        d.set_vrr(VrrPolicy::Fullscreen);
        let keys = OutputKeys {
            adaptive_sync: Some(VrrPolicy::Fullscreen),
            ..OutputKeys::default()
        };
        assert_eq!(
            flow.apply(&mut d, &mut req, false, Instant::now()),
            vec![Effect::Save("DP-1".into(), keys)]
        );
        assert!(take(&mut req).is_empty() && !flow.busy());
        assert!(!d.dirty(), "the page shows what the file now says");
        assert_eq!(d.pick().unwrap().vrr, VrrPolicy::Fullscreen);
    }

    #[test]
    fn vrr_and_mode_go_as_two_requests_vrr_first() {
        let (mut d, mut req) = smaller();
        d.set_vrr(VrrPolicy::On);
        let mut flow = Flow::default();
        flow.apply(&mut d, &mut req, true, Instant::now());
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
            flow.applied(asked[0].0, false, &mut d, &mut req, Instant::now()),
            vec![Effect::Fail(VRR_REFUSED)]
        );
        flow.applied(asked[1].0, true, &mut d, &mut req, Instant::now());
        flow.answer(true, &mut req);
        let out = flow.trial_ended(TrialEnd::Kept);
        assert!(
            !out.contains(&Effect::Quit),
            "a refused VRR keeps the sheet open: {out:?}"
        );
    }

    #[test]
    fn the_close_box_goes_back_first() {
        let (mut d, mut req) = smaller();
        let mut flow = Flow::default();
        flow.apply(&mut d, &mut req, false, Instant::now());
        let serial = take(&mut req)[0].0;
        flow.applied(serial, true, &mut d, &mut req, Instant::now());
        req.take_trial();
        assert_eq!(flow.close(&mut req), vec![]);
        assert_eq!(
            req.take_trial(),
            [TrialAsk::Revert],
            "the old mode is asked back"
        );
        assert_eq!(flow.trial_ended(TrialEnd::Reverted), vec![Effect::Quit]);
    }

    #[test]
    fn a_question_whose_output_went_away_closes_and_reverts_the_trial() {
        let mut d = Display::with_heads(vec![test_head("DP-1"), test_head("HDMI-A-1")]);
        for i in 0..2 {
            d.pick_monitor(i);
            d.pick_resolution(1);
        }
        let mut req = HeadRequests::default();
        let mut flow = Flow::default();
        flow.apply(&mut d, &mut req, false, Instant::now());
        let serial = req.take()[0].0;
        flow.applied(serial, true, &mut d, &mut req, Instant::now());
        req.take_trial();
        d.set_heads(vec![test_head("HDMI-A-1")]);
        assert_eq!(flow.heads_changed(&d, &mut req), vec![], "nothing written");
        assert!(flow.question.is_none());
        assert!(take(&mut req).is_empty());
        assert_eq!(
            req.take_trial(),
            [TrialAsk::Revert],
            "the compositor puts the output still there back"
        );
    }

    fn cancelled(
        vrr: bool,
        mode: bool,
    ) -> (Display, HeadRequests, Flow, Vec<(u32, String, HeadConfig)>) {
        let mut d = Display::with_heads(vec![test_head("DP-1")]);
        if mode {
            d.pick_resolution(1);
        }
        d.set_vrr(VrrPolicy::of(vrr));
        let mut req = HeadRequests::default();
        let mut flow = Flow::default();
        flow.apply(&mut d, &mut req, false, Instant::now());
        let asked = take(&mut req);
        assert_eq!(flow.cancel(&mut req), vec![], "waits for the compositor");
        (d, req, flow, asked)
    }

    #[test]
    fn a_mode_that_lands_after_cancel_goes_straight_back() {
        let (mut d, mut req, mut flow, asked) = cancelled(false, true);
        req.take_trial();
        assert_eq!(
            flow.applied(asked[0].0, true, &mut d, &mut req, Instant::now()),
            vec![]
        );
        assert!(flow.question.is_none(), "no question");
        assert_eq!(req.take_trial(), [TrialAsk::Revert]);
        assert_eq!(flow.trial_ended(TrialEnd::Reverted), vec![Effect::Quit]);
    }

    #[test]
    fn a_vrr_that_lands_after_cancel_goes_back_unsaved() {
        let (mut d, mut req, mut flow, asked) = cancelled(true, false);
        assert_eq!(
            flow.applied(asked[0].0, true, &mut d, &mut req, Instant::now()),
            vec![],
            "nothing saved"
        );
        let back = take(&mut req);
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].2.adaptive_sync, Some(false));
        assert_eq!(
            flow.applied(back[0].0, true, &mut d, &mut req, Instant::now()),
            vec![Effect::Quit]
        );
    }

    #[test]
    fn a_failure_after_cancel_quits_without_an_error() {
        for (vrr, mode) in [(false, true), (true, false)] {
            let (mut d, mut req, mut flow, asked) = cancelled(vrr, mode);
            assert_eq!(
                flow.applied(asked[0].0, false, &mut d, &mut req, Instant::now()),
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
    fn a_trial_that_could_not_be_put_back_says_so() {
        let (mut d, mut req) = smaller();
        let mut flow = Flow::default();
        flow.apply(&mut d, &mut req, true, Instant::now());
        let serial = take(&mut req)[0].0;
        flow.applied(serial, true, &mut d, &mut req, Instant::now());
        flow.answer(false, &mut req);
        assert_eq!(
            flow.trial_ended(TrialEnd::Failed),
            vec![Effect::Fail(
                "The old display settings could not be put back."
            )],
            "no Quit over the box"
        );
        assert!(!flow.busy());
    }

    #[test]
    fn a_vrr_put_back_after_cancel_that_fails_says_so_unless_its_output_went() {
        for gone in [false, true] {
            let (mut d, mut req, mut flow, asked) = cancelled(true, false);
            flow.applied(asked[0].0, true, &mut d, &mut req, Instant::now());
            let back = take(&mut req);
            assert_eq!(back.len(), 1, "VRR goes back as a plain request");
            if gone {
                d.set_heads(vec![test_head("HDMI-A-1")]);
            }
            let out = flow.applied(back[0].0, false, &mut d, &mut req, Instant::now());
            let fail = Effect::Fail("The old display settings could not be put back.");
            assert_eq!(out, if gone { vec![Effect::Quit] } else { vec![fail] });
            assert!(take(&mut req).is_empty(), "tried once");
            assert!(!flow.busy());
        }
    }

    #[test]
    fn yes_after_only_one_display_writes_nothing_for_the_one_turned_off() {
        let mut b = test_head("HDMI-A-1");
        b.position = (1920, 0);
        let mut d = Display::with_heads(vec![test_head("DP-1"), b]);
        d.pick_projection(2);
        let mut req = HeadRequests::default();
        let mut flow = Flow::default();
        flow.apply(&mut d, &mut req, false, Instant::now());
        let (serial, set) = req.take().remove(0);
        assert_eq!(set[1].1.enabled, Some(false));
        flow.applied(serial, true, &mut d, &mut req, Instant::now());
        flow.answer(true, &mut req);
        let out = flow.trial_ended(TrialEnd::Kept);
        let saved: Vec<&str> = (out.iter())
            .filter_map(|e| match e {
                Effect::Save(n, _) => Some(n.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(saved, ["DP-1"]);
    }
}
