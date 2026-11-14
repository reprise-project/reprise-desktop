//! When frames reach the file, in ms from the movie's start: as they come, or on a grid of `hz`
//! slots a second from the first frame. On the grid each slot codes at most one picture at its
//! own time, a change inside a coded slot is held for the next, and a fixed grid repeats the last
//! picture on a still screen.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pace {
    /// Code the frame at this pts.
    Code(i64),
    /// Keep the frame for a later [`Pacer::tick`]; a newer one replaces it.
    Hold,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pacer {
    cap_hz: Option<u32>,
    fixed: bool,
    /// The pts of the last picture coded, and in fixed mode its slot on the grid.
    last: Option<(i64, u64)>,
}

impl Pacer {
    /// `cap_hz` None codes every frame; `fixed` (with a cap) codes every slot of the grid.
    pub fn new(cap_hz: Option<u32>, fixed: bool) -> Pacer {
        let cap_hz = cap_hz.filter(|&hz| hz > 0);
        Pacer {
            cap_hz,
            fixed: fixed && cap_hz.is_some(),
            last: None,
        }
    }

    /// Forget what was coded, for a movie starting over at time 0.
    pub fn restart(&mut self) {
        self.last = None;
    }

    /// A frame that came at `at_ms`.
    pub fn frame(&mut self, at_ms: i64) -> Pace {
        let Some((last, slot)) = self.last else {
            self.last = Some((at_ms, 0));
            return Pace::Code(at_ms);
        };
        let Some(hz) = self.cap_hz else {
            return self.code(at_ms.max(last + 1), 0);
        };
        match self.due(at_ms, slot, hz) {
            Some(due) => self.code(self.at(due, hz), due),
            None => Pace::Hold,
        }
    }

    /// When the loop next wakes, in ms: the next slot while a frame is `held` or in fixed mode.
    pub fn wake(&self, held: bool) -> Option<i64> {
        let ((_, slot), hz) = (self.last?, self.cap_hz?);
        (self.fixed || held).then(|| self.at(slot + 1, hz))
    }

    /// The pts to code the `held` frame at by `now_ms`, or in fixed mode the last picture again;
    /// None when nothing is due. The grid skips the slots a slow loop missed.
    pub fn tick(&mut self, now_ms: i64, held: bool) -> Option<i64> {
        let ((_, slot), hz) = (self.last?, self.cap_hz?);
        if !(self.fixed || held) {
            return None;
        }
        let due = self.due(now_ms, slot, hz)?;
        let pts = self.at(due, hz);
        self.last = Some((pts, due));
        Some(pts)
    }

    /// The next slot, for a frame still held when the recording stops.
    pub fn flush(&mut self) -> Option<i64> {
        let ((_, slot), hz) = (self.last?, self.cap_hz?);
        let pts = self.at(slot + 1, hz);
        self.last = Some((pts, slot + 1));
        Some(pts)
    }

    /// Where a movie stopped at `now_ms` ends: the last slot on a fixed grid, else now, at least
    /// the next slot or 1 ms after the last picture.
    pub fn end(&self, now_ms: i64) -> i64 {
        match (self.last, self.cap_hz) {
            (None, _) => now_ms,
            (Some((last, _)), _) if self.fixed => last,
            (Some((_, slot)), Some(hz)) => now_ms.max(self.at(slot + 1, hz)),
            (Some((last, _)), None) => now_ms.max(last + 1),
        }
    }

    fn code(&mut self, pts: i64, slot: u64) -> Pace {
        self.last = Some((pts, slot));
        Pace::Code(pts)
    }

    /// The latest slot after `slot` that `at_ms` has reached.
    fn due(&self, at_ms: i64, slot: u64, hz: u32) -> Option<u64> {
        let origin = self.origin(slot, hz);
        let reached = ((at_ms - origin).max(0) as u64 * u64::from(hz) + 500) / 1000;
        let reached = match self.at(reached, hz) > at_ms {
            true => reached.saturating_sub(1),
            false => reached,
        };
        (reached > slot).then_some(reached)
    }

    /// Slot `k`'s pts: `round(k * 1000 / hz)` from the grid's origin.
    fn at(&self, k: u64, hz: u32) -> i64 {
        let (_, slot) = self.last.unwrap_or_default();
        self.origin(slot, hz) + offset(k, hz)
    }

    /// The grid's slot 0, from the last slot coded.
    fn origin(&self, slot: u64, hz: u32) -> i64 {
        let (last, _) = self.last.unwrap_or_default();
        last - offset(slot, hz)
    }
}

/// Slot `k`'s distance from the grid's origin.
fn offset(k: u64, hz: u32) -> i64 {
    ((k * 1000 + u64::from(hz) / 2) / u64::from(hz)) as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_inside_the_cap_is_held_not_coded() {
        let mut p = Pacer::new(Some(30), false);
        assert_eq!(p.frame(0), Pace::Code(0));
        assert_eq!(p.frame(10), Pace::Hold);
        assert_eq!(p.wake(true), Some(33));
    }

    #[test]
    fn a_change_inside_the_cap_is_coded_at_the_next_slot() {
        let mut p = Pacer::new(Some(30), false);
        p.frame(0);
        p.frame(10);
        assert_eq!(p.tick(32, true), None);
        assert_eq!(
            p.tick(33, true),
            Some(33),
            "the last change of a burst reaches the file"
        );
        assert_eq!(p.wake(false), None, "a still screen adds nothing");
        assert_eq!(p.frame(66), Pace::Hold, "slot 1 is coded");
        assert_eq!(p.frame(70), Pace::Code(67), "at slot 2's time");
    }

    #[test]
    fn a_cap_under_the_refresh_codes_one_picture_a_slot() {
        let mut p = Pacer::new(Some(144), false);
        let (mut coded, mut held) = (Vec::new(), false);
        for i in 0..330 {
            let t = (i * 1000 + 82) / 165;
            if let Some(due) = p.wake(held).filter(|&w| w <= t)
                && let Some(pts) = p.tick(due, held)
            {
                coded.push(pts);
            }
            held = match p.frame(t) {
                Pace::Code(pts) => {
                    coded.push(pts);
                    false
                }
                Pace::Hold => true,
            };
        }
        let first_second = coded.iter().filter(|&&pts| pts < 1000).count();
        assert_eq!(first_second, 144, "165 Hz arrivals at a 144 cap");
        assert!(coded.windows(2).all(|w| w[1] > w[0]));
    }

    #[test]
    fn the_fixed_rate_repeats_the_last_picture_on_a_still_screen() {
        let mut p = Pacer::new(Some(30), true);
        assert_eq!(p.frame(0), Pace::Code(0));
        let repeats: Vec<i64> = [33, 67, 100]
            .iter()
            .filter_map(|&t| p.tick(t, false))
            .collect();
        assert_eq!(repeats, [33, 67, 100]);
        assert_eq!(p.wake(false), Some(133));
    }

    #[test]
    fn the_fixed_rate_codes_a_new_frame_on_its_slot_and_each_slot_once() {
        let mut p = Pacer::new(Some(30), true);
        p.frame(0);
        assert_eq!(p.frame(20), Pace::Hold);
        assert_eq!(p.tick(32, true), None);
        assert_eq!(p.tick(33, true), Some(33));
        assert_eq!(p.tick(40, false), None, "slot 1 is coded");
        assert_eq!(p.frame(70), Pace::Code(67), "on slot 2");
        assert_eq!(p.tick(70, false), None);
        assert_eq!(p.tick(240, false), Some(233), "missed slots are skipped");
        assert_eq!(p.wake(false), Some(267));
    }

    #[test]
    fn a_frame_held_at_the_stop_takes_the_next_slot_and_the_movie_ends_a_slot_later() {
        let mut p = Pacer::new(Some(15), false);
        p.frame(0);
        p.frame(20);
        assert_eq!(p.flush(), Some(67));
        assert_eq!(p.end(70), 133, "the end in a slot of its own");
        assert_eq!(p.end(500), 500);
        let mut fixed = Pacer::new(Some(10), true);
        fixed.frame(0);
        fixed.tick(100, false);
        assert_eq!(fixed.end(150), 100, "the last slot");
        assert_eq!(fixed.flush(), Some(200));
        let mut free = Pacer::new(None, false);
        free.frame(40);
        assert_eq!((free.flush(), free.end(40)), (None, 41));
    }

    #[test]
    fn unpaced_frames_keep_their_arrival_times() {
        let mut p = Pacer::new(None, false);
        assert_eq!(
            [p.frame(0), p.frame(0), p.frame(7)],
            [Pace::Code(0), Pace::Code(1), Pace::Code(7)]
        );
        assert_eq!(p.wake(false), None);
        assert_eq!(p.tick(100, true), None);
    }

    #[test]
    fn a_restart_codes_the_next_frame_at_once() {
        let mut p = Pacer::new(Some(30), true);
        p.frame(0);
        p.restart();
        assert_eq!(p.frame(0), Pace::Code(0));
        assert_eq!(p.wake(false), Some(33));
    }
}
