use super::layout::GRIP_W;
use super::*;
use crate::raster::Rect;

impl TaskbarLayout {
    /// Rows an auto-hidden bar leaves on its output: its inner edge, `REVEAL_LINE` thick.
    pub fn reveal_line(&self) -> Rect {
        let t = self.thickness();
        let h = REVEAL_LINE.min(t).max(0);
        match self.edge {
            Edge::Bottom => Rect::new(0, 0, self.width, h),
            Edge::Top => Rect::new(0, t - h, self.width, h),
            Edge::Left => Rect::new(t - h, 0, h, self.height),
            Edge::Right => Rect::new(0, 0, h, self.height),
        }
    }

    /// The cell under (x, y); a point between the screen edge and a cell, with nothing between them, takes that cell.
    pub fn hit(&self, x: i32, y: i32) -> TaskbarHit {
        let hit = self.hit_drawn(x, y);
        let inside = |x: i32, y: i32| x >= 0 && x < self.width && y >= 0 && y < self.height;
        if hit != TaskbarHit::Nothing || !inside(x, y) {
            return hit;
        }
        let (dx, dy) = match self.edge {
            Edge::Bottom => (0, -1),
            Edge::Top => (0, 1),
            Edge::Left => (1, 0),
            Edge::Right => (-1, 0),
        };
        let (mut x, mut y) = (x + dx, y + dy);
        while inside(x, y) {
            match self.hit_drawn(x, y) {
                TaskbarHit::Nothing => (x, y) = (x + dx, y + dy),
                TaskbarHit::Resize => break,
                hit => return hit,
            }
        }
        TaskbarHit::Nothing
    }

    fn hit_drawn(&self, x: i32, y: i32) -> TaskbarHit {
        if self.disc_past_bar().is_some_and(|d| d.contains(x, y)) && self.on_disc(x, y) {
            return TaskbarHit::Start;
        }
        if y < 0 || y >= self.height || x < 0 || x >= self.width {
            return TaskbarHit::Nothing;
        }
        if self.resize.is_some_and(|r| r.contains(x, y)) {
            return TaskbarHit::Resize;
        }
        if self.start.contains(x, y) {
            return TaskbarHit::Start;
        }
        // A grip takes 2 px more each way along the bar, and the bar's
        // thickness but 2 px each side across it.
        let p = self.to_along(Rect::new(x, y, 1, 1));
        let t = self.thickness();
        for (i, g) in self.grips.iter().enumerate() {
            let g = self.to_along(*g);
            if p.x >= g.x - 2 && p.x < g.x + GRIP_W + 2 && p.y >= 2 && p.y < t - 2 {
                return TaskbarHit::Grip(i);
            }
        }
        for (i, r) in self.quick.iter().enumerate() {
            if r.contains(x, y) {
                return TaskbarHit::Quick(i);
            }
        }
        if self.quick_more.is_some_and(|r| r.contains(x, y)) {
            return TaskbarHit::QuickMore;
        }
        for (i, r) in self.pager.iter().enumerate() {
            if r.contains(x, y) {
                return TaskbarHit::Pager(i);
            }
        }
        if self.clock.contains(x, y) {
            return TaskbarHit::Clock;
        }
        if self.show_desktop.is_some_and(|r| r.contains(x, y)) {
            return TaskbarHit::ShowDesktop;
        }
        if self.chevron.is_some_and(|r| r.contains(x, y)) {
            return TaskbarHit::Chevron;
        }
        for (i, r) in self.tray.iter().enumerate() {
            if r.contains(x, y) {
                return TaskbarHit::Tray(i);
            }
        }
        for (i, r) in self.tasks.iter().enumerate() {
            if r.contains(x, y) {
                return TaskbarHit::Task(i);
            }
        }
        if self.more.is_some_and(|r| r.contains(x, y)) {
            return TaskbarHit::More;
        }
        TaskbarHit::Nothing
    }

    /// Disc rows past the bar's screen-side edge (`disc_hang`); None for a disc inside the bar.
    pub fn disc_past_bar(&self) -> Option<Rect> {
        let d = self.start_disc?;
        let y = match self.edge {
            Edge::Bottom => -self.disc_hang,
            Edge::Top => self.height,
            Edge::Left | Edge::Right => return None,
        };
        (self.disc_hang > 0).then(|| Rect::new(d.x, y, d.width, self.disc_hang))
    }

    /// Rect of the orb's own buffer: the rows past the bar and, while its menu is open, the Start rect too.
    pub fn start_orb(&self, held: bool) -> Option<Rect> {
        self.start_above
            .map(|a| if held { a.union(&self.start) } else { a })
    }

    pub fn on_disc(&self, x: i32, y: i32) -> bool {
        self.start_disc.is_some_and(|d| {
            // Doubled, so the half-pixel centres stay whole.
            let (dx, dy) = (
                2 * x + 1 - (2 * d.x + d.width),
                2 * y + 1 - (2 * d.y + d.height),
            );
            dx * dx + dy * dy <= d.width * d.width
        })
    }
}

/// Gap among `rects` that x falls in: 0 before the first, `rects.len()` after the last.
pub fn slot_at(rects: &[Rect], x: i32) -> usize {
    rects.iter().filter(|r| x >= r.x + r.width / 2).count()
}
