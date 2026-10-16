//! The copy progress dialog: what a copy or move has done and has left, Pause and Cancel. A look
//! with a [`GraphLook`] shows a throughput graph and a details toggle, the others a paper flight.

use crate::access::{self, ROOT, Tree, accesskit::Role};
use crate::classic::widgets::{BUTTON_GAP, BUTTON_H, BUTTON_W, LINE_H};
use crate::controls::form::{self, Form, Item};
use crate::copyart::{End, Flight, Graph, GraphLook, STRIP_H, Stage};
use crate::look::Paint;
use crate::look::widgets::{draw_copy_flight, draw_copy_graph, draw_progress};
use crate::raster::{Canvas, Rect};
use crate::shape;
use crate::text::{Align, TextRenderer};
use crate::theme::{Color, TextUnits};

pub const PAUSE: &str = "pause";
pub const CANCEL: &str = "cancel";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    #[default]
    Copy,
    Move,
    /// To the Recycle Bin.
    Delete,
    /// For good, past the Recycle Bin.
    Erase,
    /// Emptying the Recycle Bin.
    Empty,
    /// Out of the Recycle Bin back where it came from.
    Restore,
}

impl Kind {
    /// Whether the job is measured in items rather than bytes: a delete, Empty or Restore.
    pub fn by_items(self) -> bool {
        !matches!(self, Kind::Copy | Kind::Move)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    #[default]
    Preparing,
    Copying,
    /// Every byte is written; the destination drive is being flushed.
    Finishing,
}

/// Where the put stands, as the host reads it from the job.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
    pub phase: Phase,
    pub paused: bool,
    pub bytes: u64,
    pub total_bytes: u64,
    pub items: u64,
    pub total_items: u64,
    pub current: String,
    /// Puts queued behind this one.
    pub waiting: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    None,
    Redraw,
    /// The window changes size: the details were shown or hidden.
    Resized,
    Pause(bool),
    Cancel,
}

/// A rate is shown once the copy has run this long.
const SETTLE_MS: u64 = 1000;
/// The rate is read over at least this long.
const RATE_MS: u64 = 500;
/// The flight's frame time: it moves at about 15 frames a second. A host ticks the dialog at it.
pub const FRAME_MS: u64 = 66;
/// A put done sooner than this never shows the dialog.
pub const SHOW_AFTER: std::time::Duration = std::time::Duration::from_millis(500);
const STEPS: usize = 100;

#[derive(Clone, Debug, PartialEq)]
pub struct CopyProgress {
    pub kind: Kind,
    /// The source and destination folders' names.
    pub from: String,
    pub to: String,
    /// The graph's colour.
    pub accent: Color,
    pub status: Status,
    pub details: bool,
    rate: f64,
    last: Option<(u64, u64)>,
    copying_since: Option<u64>,
    clock: Option<u64>,
    anim_ms: u64,
    samples: Vec<f32>,
    graphed: usize,
    graph: bool,
    pub form: Form,
}

impl CopyProgress {
    pub fn new(kind: Kind, from: &str, to: &str, accent: Color) -> CopyProgress {
        let mut cp = CopyProgress {
            kind,
            from: from.into(),
            to: to.into(),
            accent,
            status: Status::default(),
            details: true,
            rate: 0.0,
            last: None,
            copying_since: None,
            clock: None,
            anim_ms: 0,
            samples: vec![0.0; STEPS + 1],
            graphed: 0,
            graph: false,
            form: Form::new(Vec::new()),
        };
        cp.form = cp.build_form();
        cp
    }

    pub fn title(&self) -> &'static str {
        match self.kind {
            Kind::Copy => "Copying...",
            Kind::Move => "Moving...",
            Kind::Delete | Kind::Erase => "Deleting...",
            Kind::Empty => "Emptying the Recycle Bin...",
            Kind::Restore => "Restoring...",
        }
    }

    pub fn fraction(&self) -> f64 {
        let s = &self.status;
        if s.phase == Phase::Finishing {
            return 1.0;
        }
        if s.total_bytes > 0 && !self.kind.by_items() {
            s.bytes as f64 / s.total_bytes as f64
        } else if s.total_items > 0 {
            s.items as f64 / s.total_items as f64
        } else {
            0.0
        }
        .clamp(0.0, 1.0)
    }

    /// What the bar measures done and in all: items for a [`Kind::by_items`] job, else bytes.
    fn measure(&self, s: &Status) -> (u64, u64) {
        if self.kind.by_items() {
            (s.items, s.total_items)
        } else {
            (s.bytes, s.total_bytes)
        }
    }

    /// Bytes (items for a [`Kind::by_items`] job) a second, once the job has run long enough to
    /// tell.
    pub fn rate(&self) -> Option<f64> {
        let settled = matches!((self.copying_since, self.clock), (Some(t), Some(now)) if now >= t + SETTLE_MS);
        (self.rate > 0.0 && settled).then_some(self.rate)
    }

    /// Take the job's `status` at `now_ms`; true when the dialog draws differently.
    pub fn update(&mut self, status: Status, now_ms: u64) -> bool {
        let running = status.phase == Phase::Copying && !status.paused;
        if running && self.copying_since.is_none() {
            self.copying_since = Some(now_ms);
        }
        let dt = self.clock.map_or(0, |c| now_ms.saturating_sub(c));
        let frame = self.anim_ms / FRAME_MS;
        if !status.paused {
            self.anim_ms += dt;
        }
        self.clock = Some(now_ms);
        if running {
            let done = self.measure(&status).0;
            match self.last {
                Some((t, b)) if now_ms >= t + RATE_MS => {
                    let inst = done.saturating_sub(b) as f64 * 1000.0 / (now_ms - t) as f64;
                    self.rate = if self.rate == 0.0 {
                        inst
                    } else {
                        0.8 * self.rate + 0.2 * inst
                    };
                    self.last = Some((now_ms, done));
                }
                Some(_) => {}
                None => self.last = Some((now_ms, done)),
            }
        } else {
            self.last = None;
        }
        let finishing = status.phase == Phase::Finishing;
        if finishing != (self.status.phase == Phase::Finishing) {
            self.form.set_enabled(PAUSE, !finishing);
        }
        let paused_changed = status.paused != self.status.paused;
        let changed = status != self.status;
        self.status = status;
        if paused_changed
            && let Some(form::Control::Button { label, .. }) =
                self.form.item_mut(PAUSE).map(|i| &mut i.control)
        {
            *label = pause_label(self.status.paused).into();
        }
        self.sample();
        changed || self.anim_ms / FRAME_MS != frame
    }

    fn sample(&mut self) {
        let Some(rate) = self.rate() else {
            return;
        };
        let upto = (self.fraction() * STEPS as f64).floor() as usize;
        if self.graphed == 0 {
            self.samples[0] = rate as f32;
        }
        while self.graphed < upto.min(STEPS) {
            self.graphed += 1;
            self.samples[self.graphed] = rate as f32;
        }
        let first_known = self.samples.iter().position(|&s| s > 0.0).unwrap_or(0);
        for i in 0..first_known {
            self.samples[i] = self.samples[first_known];
        }
    }

    /// The time remaining in words.
    pub fn remaining(&self) -> String {
        let s = &self.status;
        match s.phase {
            Phase::Preparing => return "Calculating...".into(),
            Phase::Finishing => return format!("Finishing: writing to {}...", self.to),
            Phase::Copying if s.paused => return "Paused".into(),
            Phase::Copying => {}
        }
        let (done, all) = self.measure(s);
        match self.rate() {
            Some(r) => remaining_text(all.saturating_sub(done) as f64 / r),
            None => "Calculating...".into(),
        }
    }

    /// [`CopyProgress::remaining`] without its last word, for the Time remaining row.
    fn time_left(&self) -> String {
        let r = self.remaining();
        r.strip_suffix(" remaining").map(String::from).unwrap_or(r)
    }

    fn verb(&self) -> &'static str {
        match self.kind {
            Kind::Copy => "Copying",
            Kind::Move => "Moving",
            Kind::Delete | Kind::Erase => "Deleting",
            Kind::Empty => "Emptying",
            Kind::Restore => "Restoring",
        }
    }

    fn items_text(&self) -> String {
        let n = self.status.total_items;
        let noun = if n == 1 { "item" } else { "items" };
        format!("{n} {noun}")
    }

    fn flight(&self) -> Flight {
        Flight {
            ms: self.anim_ms,
            fraction: self.fraction(),
            moving: self.kind != Kind::Copy,
            stage: match self.status.phase {
                Phase::Preparing => Stage::Preparing,
                Phase::Copying => Stage::Flying,
                Phase::Finishing => Stage::Finishing,
            },
            from: match self.kind {
                Kind::Empty | Kind::Restore => End::Bin,
                _ => End::Folder,
            },
            to: match self.kind {
                Kind::Copy | Kind::Move | Kind::Restore => End::Folder,
                Kind::Delete => End::Bin,
                Kind::Erase | Kind::Empty => End::Gone,
            },
        }
    }

    /// Follow the look `p` draws in: the graph page or the flight. Call before laying out.
    pub fn follow(&mut self, p: &Paint) {
        let graph = graph_look(p).is_some();
        if graph != self.graph {
            self.graph = graph;
            self.rebuild();
        }
    }

    pub fn toggle_details(&mut self) -> Outcome {
        if !self.graph {
            return Outcome::None;
        }
        self.details = !self.details;
        self.rebuild();
        Outcome::Resized
    }

    fn rebuild(&mut self) {
        let mut f = self.build_form();
        f.carry(&self.form);
        self.form = f;
    }

    /// The client size at 100% text.
    pub fn client(&self) -> (i32, i32) {
        if self.graph {
            (GLASS_W, self.band_y() + BAND_H)
        } else {
            (W, FLAT_BUTTONS_Y + BUTTON_H + MARGIN)
        }
    }

    fn width(&self) -> i32 {
        if self.graph { GLASS_W } else { W }
    }

    /// Where the buttons' row starts: the band's top on the graph page.
    fn band_y(&self) -> i32 {
        match (self.graph, self.details) {
            (false, _) => FLAT_BUTTONS_Y,
            (true, true) => DETAILS_BAND_Y,
            (true, false) => FEWER_BAND_Y,
        }
    }

    fn buttons_y(&self) -> i32 {
        if self.graph {
            self.band_y() + (BAND_H - BUTTON_H) / 2
        } else {
            self.band_y()
        }
    }

    fn build_form(&self) -> Form {
        let w = self.width();
        let y = self.buttons_y();
        let cancel_x = w - MARGIN - BUTTON_W;
        let pause_x = cancel_x - BUTTON_GAP - BUTTON_W;
        let pause = pause_label(self.status.paused);
        let mut f = Form::new(vec![
            Item::button(
                PAUSE,
                Rect::new(pause_x, y, BUTTON_W, BUTTON_H),
                pause,
                self.status.phase != Phase::Finishing,
                false,
            ),
            Item::button(
                CANCEL,
                Rect::new(cancel_x, y, BUTTON_W, BUTTON_H),
                "Cancel",
                true,
                true,
            ),
        ]);
        f.focus_id(CANCEL);
        f
    }

    /// A form event read as what the dialog does: Escape and the close box cancel.
    pub fn event(&mut self, ev: form::Event) -> Outcome {
        match ev {
            form::Event::Pressed(PAUSE) => Outcome::Pause(!self.status.paused),
            form::Event::Pressed(CANCEL) | form::Event::Cancel | form::Event::Default => {
                Outcome::Cancel
            }
            form::Event::Redraw | form::Event::Changed(_) => Outcome::Redraw,
            _ => Outcome::None,
        }
    }

    /// The details toggle's rect at 100%, on the graph page only.
    fn toggle_rect(&self) -> Option<Rect> {
        self.graph
            .then(|| Rect::new(MARGIN, self.buttons_y() + 3, TOGGLE_W, TOGGLE_ICON))
    }

    /// Whether (`x`, `y`) is on the details toggle.
    pub fn on_toggle(&self, x: i32, y: i32, u: TextUnits) -> bool {
        self.toggle_rect().is_some_and(|r| u.rect(r).contains(x, y))
    }

    /// A press at (`x`, `y`) on the details toggle.
    pub fn press(&mut self, x: i32, y: i32, u: TextUnits) -> Option<Outcome> {
        self.on_toggle(x, y, u).then(|| self.toggle_details())
    }

    /// The balloon's summary and body for a job that dealt with `landed` items.
    pub fn ended_note(&self, landed: usize) -> (String, String) {
        let noun = if landed == 1 { "item" } else { "items" };
        let (verb, place) = match self.kind {
            Kind::Copy => ("Copied", format!("to {}", self.to)),
            Kind::Move => ("Moved", format!("to {}", self.to)),
            Kind::Delete | Kind::Erase => ("Deleted", format!("from {}", self.from)),
            Kind::Restore => ("Restored", format!("from {}", self.from)),
            Kind::Empty => {
                return (
                    "Emptied the Recycle Bin".into(),
                    format!("Deleted {landed} {noun}"),
                );
            }
        };
        (
            format!("{verb} {landed} {noun}"),
            format!("{verb} {landed} {noun} {place}"),
        )
    }

    pub fn draw(&self, c: &mut Canvas, text: &TextRenderer, p: &Paint, px: u32) {
        match graph_look(p) {
            Some(look) if self.graph => self.draw_page(c, text, p, &look, px),
            _ => self.draw_flight(c, text, p, px),
        }
        form::draw(c, text, p, &self.form, true, px);
    }

    fn draw_flight(&self, c: &mut Canvas, text: &TextRenderer, p: &Paint, px: u32) {
        let u = p.text;
        let cs = p.cs;
        c.fill_rect(c.bounds(), cs.button_face);
        let strip = u.at(Rect::new(MARGIN, STRIP_Y, u.len(W - 2 * MARGIN), STRIP_H));
        draw_copy_flight(c, p, strip, &self.flight());
        let line = |y: i32| u.rect(Rect::new(MARGIN, y, W - 2 * MARGIN, LINE_H + 2));
        let name = match (self.status.phase, self.kind) {
            (Phase::Preparing, Kind::Copy) => "Preparing to copy...".to_string(),
            (Phase::Preparing, Kind::Move) => "Preparing to move...".to_string(),
            (Phase::Preparing, Kind::Restore) => "Preparing to restore...".to_string(),
            (Phase::Preparing, _) => "Preparing to delete...".to_string(),
            _ => self.status.current.clone(),
        };
        let ink = cs.button_text;
        let r = line(NAME_Y);
        text.draw(
            c,
            r,
            &text.ellipsize(&name, r.width, false, px),
            false,
            ink,
            Align::Left,
            px,
        );
        let r = line(FROM_Y);
        let from_to = if self.kind.by_items() {
            format!("From '{}'", self.from)
        } else {
            format!("From '{}' to '{}'", self.from, self.to)
        };
        text.draw(
            c,
            r,
            &text.ellipsize(&from_to, r.width, false, px),
            false,
            ink,
            Align::Left,
            px,
        );
        let bar = u.rect(Rect::new(MARGIN, BAR_Y, W - 2 * MARGIN, BAR_H));
        self.draw_bar(c, p, bar);
        let r = line(LEFT_Y);
        text.draw(c, r, &self.remaining(), false, ink, Align::Left, px);
        if self.status.waiting > 0 {
            let more = format!("{} more waiting", self.status.waiting);
            text.draw(c, r, &more, false, cs.gray_text, Align::Right, px);
        }
    }

    /// The bar, or while the drive is flushed a block sweeping across it.
    fn draw_bar(&self, c: &mut Canvas, p: &Paint, bar: Rect) {
        if self.status.phase != Phase::Finishing {
            draw_progress(c, p, bar, self.fraction(), !self.status.paused);
            return;
        }
        draw_progress(c, p, bar, 0.0, true);
        let sweep = bar.width / 4;
        let t = (self.anim_ms % SWEEP_MS) as f64 / SWEEP_MS as f64;
        let x = bar.x - sweep + ((bar.width + sweep) as f64 * shape::smooth(t)) as i32;
        let clip = Rect::new(x, bar.y, sweep, bar.height).intersect(&bar.inset(2));
        let s = p.s;
        let mut full =
            Canvas::with_scale((bar.width * s) as u32, (bar.height * s) as u32, c.scale());
        draw_progress(
            &mut full,
            p,
            Rect::new(0, 0, bar.width, bar.height),
            1.0,
            true,
        );
        c.with_clip(clip.scaled(s), |c| {
            c.blit_canvas(bar.x * s, bar.y * s, &full)
        });
    }

    fn draw_page(&self, c: &mut Canvas, text: &TextRenderer, p: &Paint, look: &GraphLook, px: u32) {
        let u = p.text;
        let cs = p.cs;
        let w = GLASS_W - 2 * GLASS_MARGIN;
        c.fill_rect(c.bounds(), cs.window);
        let band = u.rect(Rect::new(0, self.band_y(), GLASS_W, BAND_H));
        let band = Rect::new(0, band.y, c.width as i32, c.height as i32 - band.y);
        c.fill_rect(band, look.band);
        c.fill_rect(Rect::new(0, band.y, band.width, 1), look.band_rule);
        let header = u.rect(Rect::new(GLASS_MARGIN, HEADER_Y, w, LINE_H + 2));
        self.draw_header(c, text, p, header, px);
        let big_px = px * 3 / 2;
        let big = u.rect(Rect::new(GLASS_MARGIN, BIG_Y, w, BIG_H));
        let pct = (self.fraction() * 100.0).floor() as u32;
        let heading = match (self.status.phase, self.status.paused) {
            (Phase::Preparing, _) => "Calculating...".to_string(),
            (Phase::Finishing, _) => "Finishing...".to_string(),
            (_, true) => format!("Paused - {pct}% complete"),
            _ => format!("{pct}% complete"),
        };
        text.draw(c, big, &heading, false, look.heading, Align::Left, big_px);
        if self.status.waiting > 0 {
            let more = format!("{} more waiting", self.status.waiting);
            text.draw(c, big, &more, false, cs.gray_text, Align::Right, px);
        }
        if self.details {
            let g = u.rect(Rect::new(GLASS_MARGIN, GRAPH_Y, w, GRAPH_H));
            let graph = Graph {
                samples: &self.samples[..=self.graphed],
                fraction: self.fraction(),
                rate: self.rate().unwrap_or(0.0),
                accent: self.accent,
            };
            draw_copy_graph(c, p, g, &graph);
            if let Some(r) = self.rate() {
                let label = if self.kind.by_items() {
                    {
                        let n = r.round() as u64;
                        format!("Speed: {n} {}/s", if n == 1 { "item" } else { "items" })
                    }
                } else {
                    format!("Speed: {}/s", bytes_text(r as u64))
                };
                let lw = text.measure(&label, false, px) + 6;
                let lh = u.len(LINE_H + 2);
                let ry = g.y + graph.rule_y(g.height).round() as i32;
                let ly = (ry - lh - 1).max(g.y + 2);
                let lr = Rect::new(g.right() - lw - 3, ly, lw, lh);
                c.fill_rect(lr, cs.window);
                text.draw(c, lr, &label, false, cs.window_text, Align::Center, px);
            }
            let rows = [
                ("Name:", self.status.current.clone()),
                ("Time remaining:", self.time_left()),
                ("Items remaining:", self.items_left()),
            ];
            for (i, (label, value)) in rows.iter().enumerate() {
                let y = ROWS_Y + i as i32 * ROW_H;
                let l = u.rect(Rect::new(GLASS_MARGIN, y, LABEL_W, LINE_H + 2));
                let v = u.rect(Rect::new(
                    GLASS_MARGIN + LABEL_W,
                    y,
                    w - LABEL_W,
                    LINE_H + 2,
                ));
                text.draw(c, l, label, false, cs.gray_text, Align::Left, px);
                text.draw(
                    c,
                    v,
                    &text.ellipsize(value, v.width, false, px),
                    false,
                    cs.window_text,
                    Align::Left,
                    px,
                );
            }
        } else {
            let bar = u.rect(Rect::new(GLASS_MARGIN, FEWER_BAR_Y, w, BAR_H));
            self.draw_bar(c, p, bar);
            let r = u.rect(Rect::new(
                GLASS_MARGIN,
                FEWER_BAR_Y + BAR_H + 6,
                w,
                LINE_H + 2,
            ));
            text.draw(
                c,
                r,
                &self.remaining(),
                false,
                cs.window_text,
                Align::Left,
                px,
            );
        }
        if let Some(t) = self.toggle_rect() {
            self.draw_toggle(c, text, p, u.rect(t), px);
        }
    }

    fn items_left(&self) -> String {
        let s = &self.status;
        let n = s.total_items.saturating_sub(s.items);
        if self.kind.by_items() {
            return n.to_string();
        }
        format!(
            "{n} ({})",
            bytes_text(s.total_bytes.saturating_sub(s.bytes))
        )
    }

    /// "Copying 12 items (1.2 GB) from Downloads to KINGSTON", the folders in the link ink.
    fn draw_header(&self, c: &mut Canvas, text: &TextRenderer, p: &Paint, r: Rect, px: u32) {
        let lead = match self.status.phase {
            _ if self.kind.by_items() => format!("{} {} from ", self.verb(), self.items_text()),
            Phase::Preparing => format!("{} {} from ", self.verb(), self.items_text()),
            _ => format!(
                "{} {} ({}) from ",
                self.verb(),
                self.items_text(),
                bytes_text(self.status.total_bytes)
            ),
        };
        let to = if self.kind.by_items() { "" } else { " to " };
        let fixed = text.measure(&lead, false, px) + text.measure(to, false, px);
        let each = if to.is_empty() {
            (r.width - fixed).max(0)
        } else {
            ((r.width - fixed) / 2).max(0)
        };
        let from = text.ellipsize(&self.from, each, false, px);
        let dest = if to.is_empty() {
            String::new()
        } else {
            text.ellipsize(&self.to, each, false, px).into_owned()
        };
        let mut x = r.x;
        for (s, ink) in [
            (lead.as_str(), p.cs.window_text),
            (&from, p.cs.hot_light),
            (to, p.cs.window_text),
            (&dest, p.cs.hot_light),
        ] {
            let w = text.measure(s, false, px);
            text.draw(
                c,
                Rect::new(x, r.y, w + 1, r.height),
                s,
                false,
                ink,
                Align::Left,
                px,
            );
            x += w;
        }
    }

    /// A round button with a chevron, pointing down to show the details and up to hide them.
    fn draw_toggle(&self, c: &mut Canvas, text: &TextRenderer, p: &Paint, r: Rect, px: u32) {
        let side = r.height;
        let d = p.s * c.scale();
        let k = f64::from(d);
        let ring = shape::mix(premul_of(p.cs.window), premul_of(p.cs.window_text), 0.35);
        let face = shape::mix(premul_of(p.cs.window), premul_of(p.cs.button_face), 0.5);
        let ink = premul_of(p.cs.window_text);
        let up = self.details;
        let n = side * d;
        let s = f64::from(side);
        let chevron: Vec<(f64, f64)> = {
            let (a, b) = if up { (0.62, 0.40) } else { (0.40, 0.62) };
            [(0.30, a), (0.50, b), (0.70, a)]
                .iter()
                .map(|&(x, y)| (x * s * k, y * s * k))
                .collect()
        };
        let art = shape::draw(side, side, d, |x, y| {
            let c0 = s * k / 2.0;
            let outer = shape::ellipse(x, y, c0, c0, c0 - 0.5 * k, c0 - 0.5 * k);
            let inner = shape::ellipse(x, y, c0, c0, c0 - 1.5 * k, c0 - 1.5 * k);
            let mark = shape::stroke(x, y, &chevron, 1.6 * k);
            let base = shape::shares(&[(ring, outer - inner), (face, inner)]);
            shape::over(shape::scale(ink, mark), base)
        });
        debug_assert_eq!(art.width as i32, n);
        c.blit_over(r.x * p.s, r.y * p.s, &art.at_scale(c.scale()));
        let label = if self.details {
            "Fewer &details"
        } else {
            "More &details"
        };
        let (plain, key) = crate::text::split_mnemonic(label);
        let t = Rect::new(r.x + side + 6, r.y, r.width - side - 6, side);
        text.draw(c, t, &plain, false, p.cs.window_text, Align::Left, px);
        if let Some(i) = key {
            let (x0, x1) = text.span(&plain, i, false, px);
            if let Some(b) = text.baseline(t, false, px) {
                c.fill_rect(Rect::new(t.x + x0, b + 1, x1 - x0, 1), p.cs.window_text);
            }
        }
    }

    pub fn access(&self, tree: &mut Tree, text: &TextRenderer, p: &Paint, px: u32) {
        let u = p.text;
        let (w, h) = u.size(self.client());
        let all = Rect::new(0, 0, w, h);
        let lines = [
            self.status.current.clone(),
            format!("From {} to {}", self.from, self.to),
            self.remaining(),
        ];
        for (i, l) in lines.iter().enumerate() {
            tree.push(ROOT, ("line", i), access::label(all, l, true));
        }
        let mut bar = access::range(
            Role::ProgressIndicator,
            all,
            self.fraction() * 100.0,
            0.0,
            100.0,
            true,
        );
        bar.set_label(self.title());
        tree.push(ROOT, "progress", bar);
        if let Some(t) = self.toggle_rect() {
            let label = if self.details {
                "Fewer details"
            } else {
                "More details"
            };
            tree.push(
                ROOT,
                "details",
                access::button(u.rect(t), label, true, false),
            );
        }
        if let Some(f) = self.form.access(tree, ROOT, text, px) {
            tree.set_focus(f);
        }
    }
}

fn pause_label(paused: bool) -> &'static str {
    if paused { "&Resume" } else { "&Pause" }
}

fn premul_of(c: Color) -> shape::Px {
    shape::premul(c, 255)
}

/// Whether `p`'s look draws the graph page.
pub fn graph_look(p: &Paint) -> Option<GraphLook> {
    p.look.style().and_then(|st| st.copy_graph)
}

const W: i32 = 360;
const MARGIN: i32 = 10;
const STRIP_Y: i32 = 6;
const NAME_Y: i32 = 70;
const FROM_Y: i32 = 86;
const BAR_Y: i32 = 106;
const BAR_H: i32 = 16;
const LEFT_Y: i32 = 128;
const FLAT_BUTTONS_Y: i32 = 150;

const GLASS_W: i32 = 440;
const GLASS_MARGIN: i32 = 16;
const HEADER_Y: i32 = 12;
const BIG_Y: i32 = 30;
const BIG_H: i32 = 24;
const GRAPH_Y: i32 = 62;
const GRAPH_H: i32 = 84;
const ROWS_Y: i32 = 156;
const ROW_H: i32 = 18;
const LABEL_W: i32 = 110;
const DETAILS_BAND_Y: i32 = 216;
const FEWER_BAR_Y: i32 = 64;
const FEWER_BAND_Y: i32 = 112;
const BAND_H: i32 = 44;
const TOGGLE_ICON: i32 = 17;
const TOGGLE_W: i32 = 130;
const SWEEP_MS: u64 = 1400;

/// A size in bytes, three figures past a kilobyte, cut not rounded: "512 bytes", "1.23 KB",
/// "12.3 MB", "123 GB".
pub fn bytes_text(bytes: u64) -> String {
    if bytes < 1024 {
        return format!("{bytes} bytes");
    }
    let units = ["KB", "MB", "GB", "TB"];
    let mut div: u128 = 1024;
    let mut unit = 0;
    while u128::from(bytes) / div >= 1000 && unit + 1 < units.len() {
        div *= 1024;
        unit += 1;
    }
    let h = u128::from(bytes) * 100 / div;
    let n = if h < 1_000 {
        format!("{}.{:02}", h / 100, h % 100)
    } else if h < 10_000 {
        format!("{}.{}", h / 100, h % 100 / 10)
    } else {
        format!("{}", h / 100)
    };
    format!("{n} {}", units[unit])
}

/// `secs` left in words: "A few seconds", seconds in fives under a minute, then minutes, then
/// hours and minutes.
pub fn remaining_text(secs: f64) -> String {
    let secs = secs.max(0.0).ceil() as u64;
    let plural = |n: u64, one: &str| {
        if n == 1 {
            format!("1 {one}")
        } else {
            format!("{n} {one}s")
        }
    };
    let body = if secs <= 5 {
        return "A few seconds remaining".into();
    } else if secs < 60 {
        plural(secs.div_ceil(5) * 5, "second")
    } else if secs < 3600 {
        plural(((secs + 30) / 60).max(1), "minute")
    } else {
        let (h, m) = (secs / 3600, (secs % 3600 + 30) / 60);
        match m {
            0 => plural(h, "hour"),
            m => format!("{} and {}", plural(h, "hour"), plural(m, "minute")),
        }
    };
    format!("About {body} remaining")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blue;
    use crate::interface::Interface;
    use crate::interfaces::{blue_theme, theme_for};
    use crate::theme::{FontConfig, Theme};

    const SKY: Color = Color::rgb(0x2E, 0x8C, 0xFF);

    fn copying(bytes: u64, total: u64) -> Status {
        Status {
            phase: Phase::Copying,
            bytes,
            total_bytes: total,
            items: 3,
            total_items: 12,
            current: "holiday.mkv".into(),
            ..Status::default()
        }
    }

    #[test]
    fn a_delete_is_measured_in_items_and_named_by_its_kind() {
        let mut cp = CopyProgress::new(Kind::Delete, "Downloads", "", SKY);
        let at = |items| Status {
            phase: Phase::Copying,
            items,
            total_items: 1000,
            bytes: 900,
            total_bytes: 1000,
            ..Status::default()
        };
        cp.update(at(0), 0);
        cp.update(at(100), 1000);
        cp.update(at(200), 2000);
        assert_eq!(cp.fraction(), 0.2, "items, not bytes");
        assert_eq!(cp.remaining(), "About 10 seconds remaining");
        assert_eq!(cp.title(), "Deleting...");
        assert_eq!(
            CopyProgress::new(Kind::Empty, "a", "", SKY).title(),
            "Emptying the Recycle Bin..."
        );
        assert_eq!(
            CopyProgress::new(Kind::Restore, "a", "", SKY).title(),
            "Restoring..."
        );
        assert_eq!(
            cp.ended_note(2),
            (
                "Deleted 2 items".to_string(),
                "Deleted 2 items from Downloads".to_string()
            )
        );
    }

    fn fnv(c: &Canvas) -> u64 {
        c.pixels.iter().fold(0xcbf2_9ce4_8422_2325, |h, &p| {
            (h ^ u64::from(p)).wrapping_mul(0x0100_0000_01b3)
        })
    }

    #[test]
    fn sizes_keep_three_figures() {
        assert_eq!(bytes_text(512), "512 bytes");
        assert_eq!(bytes_text(1260), "1.23 KB");
        assert_eq!(bytes_text(12_900_000), "12.3 MB");
        assert_eq!(bytes_text(1_280_000_000), "1.19 GB");
    }

    #[test]
    fn the_time_left_reads_in_rounded_words() {
        assert_eq!(remaining_text(3.0), "A few seconds remaining");
        assert_eq!(remaining_text(21.0), "About 25 seconds remaining");
        assert_eq!(remaining_text(65.0), "About 1 minute remaining");
        assert_eq!(remaining_text(130.0), "About 2 minutes remaining");
        assert_eq!(remaining_text(3600.0), "About 1 hour remaining");
        assert_eq!(
            remaining_text(7500.0),
            "About 2 hours and 5 minutes remaining"
        );
    }

    #[test]
    fn the_rate_waits_a_second_then_follows_the_bytes() {
        let mut cp = CopyProgress::new(Kind::Copy, "Downloads", "KINGSTON", SKY);
        cp.update(copying(0, 100_000_000), 0);
        cp.update(copying(4_000_000, 100_000_000), 500);
        assert_eq!(cp.remaining(), "Calculating...", "too soon to tell");
        cp.update(copying(8_000_000, 100_000_000), 1000);
        let rate = cp.rate().expect("a rate after a second");
        assert!((rate - 8.0e6).abs() < 1.0, "8 MB a second, not {rate}");
        assert_eq!(cp.remaining(), "About 15 seconds remaining");
    }

    #[test]
    fn a_pause_holds_the_flight_and_drops_no_rate() {
        let mut cp = CopyProgress::new(Kind::Copy, "a", "b", SKY);
        cp.update(copying(0, 100), 0);
        cp.update(copying(50, 100), 1000);
        let flown = cp.anim_ms;
        let paused = Status {
            paused: true,
            ..copying(50, 100)
        };
        cp.update(paused.clone(), 1500);
        cp.update(paused, 9000);
        assert_eq!(cp.anim_ms, flown, "the sheets stop where they are");
        assert_eq!(cp.remaining(), "Paused");
        assert!(cp.event(form::Event::Pressed(PAUSE)) == Outcome::Pause(false));
        let label = |cp: &CopyProgress| match &cp.form.item(PAUSE).unwrap().control {
            form::Control::Button { label, .. } => label.clone(),
            other => panic!("{other:?}"),
        };
        assert_eq!(label(&cp), "&Resume");
        cp.update(copying(50, 100), 9100);
        assert_eq!(label(&cp), "&Pause");
    }

    #[test]
    fn finishing_fills_the_bar_and_names_the_drive() {
        let mut cp = CopyProgress::new(Kind::Move, "a", "KINGSTON", SKY);
        cp.update(
            Status {
                phase: Phase::Finishing,
                ..copying(90, 100)
            },
            0,
        );
        assert_eq!(cp.fraction(), 1.0);
        assert_eq!(cp.remaining(), "Finishing: writing to KINGSTON...");
        assert_eq!(cp.title(), "Moving...");
    }

    #[test]
    fn the_graph_look_lays_out_a_page_and_the_others_the_flight() {
        let mut cp = CopyProgress::new(Kind::Copy, "a", "b", SKY);
        cp.follow(&Theme::default().paint());
        assert_eq!(cp.client(), (360, 183));
        let cancel = cp.form.item(CANCEL).unwrap().rect;
        assert_eq!((cancel.right(), cancel.bottom()), (350, 173));
        let glass = theme_for(Interface::Glass);
        cp.follow(&glass.paint());
        assert_eq!(cp.client(), (440, 260));
        assert_eq!(cp.toggle_details(), Outcome::Resized);
        assert_eq!(cp.client(), (440, 156));
        let cancel = cp.form.item(CANCEL).unwrap().rect;
        assert_eq!(cancel.bottom() + 11, 156, "Cancel sits in the band");
        let u = TextUnits::ONE;
        let t = cp.toggle_rect().unwrap();
        assert_eq!(cp.press(t.x + 4, t.y + 4, u), Some(Outcome::Resized));
        assert!(cp.details);
        cp.follow(&blue_theme(blue::Variant::Olive).paint());
        assert_eq!(
            cp.toggle_details(),
            Outcome::None,
            "no details without the graph"
        );
    }

    #[test]
    fn escape_and_cancel_cancel() {
        let mut cp = CopyProgress::new(Kind::Copy, "a", "b", SKY);
        assert_eq!(cp.event(form::Event::Cancel), Outcome::Cancel);
        assert_eq!(cp.event(form::Event::Pressed(CANCEL)), Outcome::Cancel);
        assert_eq!(
            cp.form.default_button().map(|i| cp.form.items[i].id),
            Some(CANCEL)
        );
    }

    /// The flight in each look and Blue scheme, each stage, at 1 and 2.
    #[test]
    fn the_flight_is_pinned_per_look_scheme_and_stage() {
        let mut themes = vec![("classic".to_string(), Theme::default())];
        for v in blue::Variant::ALL {
            themes.push((format!("blue {v:?}"), blue_theme(v)));
        }
        let stages = [
            ("preparing", Stage::Preparing, 0.0),
            ("copying", Stage::Flying, 0.4),
            ("finishing", Stage::Finishing, 1.0),
        ];
        let ops = [
            ("copy", false, End::Folder, End::Folder),
            ("move", true, End::Folder, End::Folder),
            ("delete", true, End::Folder, End::Bin),
            ("erase", true, End::Folder, End::Gone),
            ("empty", true, End::Bin, End::Gone),
            ("restore", true, End::Bin, End::Folder),
        ];
        let mut got = Vec::new();
        for (name, theme) in &themes {
            for (stage_name, stage, fraction) in stages {
                for (op, moving, from, to) in ops {
                    for s in [1, 2] {
                        let mut c = Canvas::with_scale(340, STRIP_H as u32, s);
                        let f = Flight {
                            ms: 420,
                            fraction,
                            moving,
                            stage,
                            from,
                            to,
                        };
                        draw_copy_flight(&mut c, &theme.paint(), Rect::new(0, 0, 340, STRIP_H), &f);
                        got.push((format!("{name} {stage_name} {op} @{s}"), fnv(&c)));
                    }
                }
            }
        }
        crate::pins::check("copy_flight", &got);
        let distinct: std::collections::HashSet<_> = got.iter().map(|g| g.1).collect();
        assert!(distinct.len() > got.len() * 3 / 4, "the states draw apart");
    }

    #[test]
    fn every_look_draws_the_dialog_inside_its_face() {
        let text = TextRenderer::load(&FontConfig::default());
        for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
            let theme = theme_for(i);
            let p = theme.paint();
            let mut cp = CopyProgress::new(Kind::Copy, "Downloads", "KINGSTON", SKY);
            cp.follow(&p);
            cp.update(copying(0, 1 << 30), 0);
            cp.update(copying(1 << 28, 1 << 30), 2000);
            let (w, h) = cp.client();
            let mut c = Canvas::new(w as u32, h as u32);
            cp.draw(&mut c, &text, &p, theme.fonts.size_px);
            let face = if i == Interface::Glass {
                graph_look(&p).unwrap().band
            } else {
                theme.colors.button_face
            };
            assert_eq!(
                c.get(1, h - 2),
                face.argb(),
                "{i:?}: the bottom is the face"
            );
            let bar_or_graph = (20..w - 20)
                .filter(|&x| c.get(x, 110) != c.get(x, 2))
                .count();
            assert!(
                bar_or_graph > 40,
                "{i:?}: something is drawn across the middle"
            );
        }
    }

    /// Writes target/copyprogress-*.png for a visual check.
    #[test]
    #[ignore]
    fn render_copyprogress() {
        let text = TextRenderer::load(&FontConfig::default());
        for (name, theme) in [
            ("classic", Theme::default()),
            ("blue", theme_for(Interface::Blue)),
            ("glass", theme_for(Interface::Glass)),
        ] {
            for s in [1, 2] {
                let p = theme.paint();
                let mut cp = CopyProgress::new(Kind::Move, "Downloads", "KINGSTON", SKY);
                cp.follow(&p);
                for k in 0..40u64 {
                    let b = k * 9_000_000 + (k % 7) * 2_000_000;
                    cp.update(copying(b, 900_000_000), k * 300);
                }
                let (w, h) = cp.client();
                let mut c = Canvas::with_scale(w as u32, h as u32, s);
                cp.draw(&mut c, &text, &p, theme.fonts.size_px);
                crate::raster::write_preview(&c, &format!("copyprogress-{name}@{s}.png"));
            }
        }
    }
}
