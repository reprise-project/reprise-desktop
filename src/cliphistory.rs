//! The clipboard history: every new selection is recorded in memory, newest first, for the Super+V popup (`clipview`).
//! A selection is read over a pipe one MIME type at a time; a `secret` password-manager hint ends the read with nothing kept.
//! A newer selection abandons the read in flight. Nothing is recorded while locked or with `[clipboard] history = false`.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use reprisekit::raster::Canvas;
use smithay::reexports::calloop::{
    Interest, Mode, PostAction, RegistrationToken,
    generic::Generic,
    timer::{TimeoutAction, Timer},
};
use smithay::wayland::selection::data_device::data_device_selection_alive;

use crate::clipboard::{Offer, TEXT_MIMES};
use crate::compositor::State;

pub const MAX_ENTRIES: usize = 25;
pub const MAX_BYTES: usize = 10 << 20;
const EXTRA_MAX: usize = 64 << 10;
const MAX_EXTRAS: usize = 8;
const STEP_TIMEOUT: Duration = Duration::from_secs(2);
/// A password manager's mark on a selection; the value `secret` keeps it
/// out of the history.
const HINT: &str = "x-kde-passwordManagerHint";
const PNG: &str = "image/png";
const THUMB: u32 = 32;
const THUMB_MAX_SIDE: u32 = 8192;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Text(String),
    Image { width: u32, height: u32 },
}

impl Kind {
    pub fn label(&self) -> String {
        match self {
            Kind::Text(preview) => preview.clone(),
            Kind::Image { width, height } => format!("Image, {width}x{height}"),
        }
    }
}

pub struct Entry {
    pub id: u64,
    pub kind: Kind,
    primary: Arc<[u8]>,
    pub offers: Vec<Offer>,
    pub thumb: Option<Canvas>,
}

impl Entry {
    fn size(&self) -> usize {
        let mut seen: Vec<*const u8> = Vec::new();
        let mut n = 0;
        for (_, b) in &self.offers {
            if !seen.contains(&b.as_ptr()) {
                seen.push(b.as_ptr());
                n += b.len();
            }
        }
        n
    }
}

#[derive(Default)]
pub struct History {
    entries: VecDeque<Entry>,
    next_id: u64,
}

impl History {
    pub fn entries(&self) -> impl Iterator<Item = &Entry> {
        self.entries.iter()
    }

    pub fn get(&self, id: u64) -> Option<&Entry> {
        self.entries.iter().find(|e| e.id == id)
    }

    pub fn get_mut(&mut self, id: u64) -> Option<&mut Entry> {
        self.entries.iter_mut().find(|e| e.id == id)
    }

    pub fn remove(&mut self, id: u64) {
        self.entries.retain(|e| e.id != id);
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn push(&mut self, kind: Kind, primary: Arc<[u8]>, offers: Vec<Offer>) -> Option<u64> {
        let is_image = matches!(kind, Kind::Image { .. });
        let old = self.entries.iter().position(|e| {
            matches!(e.kind, Kind::Image { .. }) == is_image && e.primary[..] == primary[..]
        });
        let (id, thumb) = match old.and_then(|i| self.entries.remove(i)) {
            Some(e) => (e.id, e.thumb),
            None => {
                self.next_id += 1;
                (self.next_id, None)
            }
        };
        let entry = Entry {
            id,
            kind,
            primary,
            offers,
            thumb,
        };
        if entry.size() > MAX_BYTES {
            return None;
        }
        self.entries.push_front(entry);
        self.entries.truncate(MAX_ENTRIES);
        while self.entries.iter().map(Entry::size).sum::<usize>() > MAX_BYTES {
            self.entries.pop_back();
        }
        Some(id)
    }
}

pub fn preview(text: &str) -> Option<String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() {
        return None;
    }
    Some(words.join(" ").chars().take(120).collect())
}

pub fn png_size(bytes: &[u8]) -> Option<(u32, u32)> {
    const SIG: &[u8] = b"\x89PNG\r\n\x1a\n";
    if bytes.len() < 24 || &bytes[..8] != SIG || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let be = |at: usize| u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap());
    let (w, h) = (be(16), be(20));
    (w > 0 && h > 0).then_some((w, h))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    Hint,
    Text(String),
    Png,
    Extra(String),
}

impl Step {
    fn mime(&self) -> &str {
        match self {
            Step::Hint => HINT,
            Step::Text(m) | Step::Extra(m) => m,
            Step::Png => PNG,
        }
    }

    fn limit(&self) -> usize {
        match self {
            Step::Hint => 64,
            Step::Text(_) | Step::Png => MAX_BYTES,
            Step::Extra(_) => EXTRA_MAX,
        }
    }
}

pub fn plan(offered: &[String]) -> Vec<Step> {
    let has = |m: &str| offered.iter().any(|o| o == m);
    let text = TEXT_MIMES.iter().find(|m| has(m));
    if text.is_none() && !has(PNG) {
        return Vec::new();
    }
    let mut steps = Vec::new();
    if has(HINT) {
        steps.push(Step::Hint);
    }
    if let Some(m) = text {
        steps.push(Step::Text(m.to_string()));
    }
    if has(PNG) {
        steps.push(Step::Png);
    }
    let mut extras: Vec<&String> = Vec::new();
    for o in offered {
        let skip = !o.contains('/')
            || o.starts_with("text/plain")
            || o == PNG
            || o == HINT
            || extras.contains(&o);
        if !skip && extras.len() < MAX_EXTRAS {
            extras.push(o);
        }
    }
    steps.extend(extras.into_iter().map(|o| Step::Extra(o.clone())));
    steps
}

#[derive(Default)]
struct Got {
    text: Option<Arc<[u8]>>,
    png: Option<Arc<[u8]>>,
    extras: Vec<Offer>,
}

impl Got {
    fn from_offers(offers: &[Offer]) -> Got {
        let mut got = Got::default();
        for (mime, bytes) in offers {
            if TEXT_MIMES.contains(&mime.as_str()) {
                got.text.get_or_insert(bytes.clone());
            } else if mime == PNG {
                got.png = Some(bytes.clone());
            } else if mime != HINT {
                got.extras.push((mime.clone(), bytes.clone()));
            }
        }
        got
    }

    fn into_entry(self) -> Option<(Kind, Arc<[u8]>, Vec<Offer>)> {
        let text = self.text.and_then(|t| {
            let preview = preview(&String::from_utf8_lossy(&t))?;
            Some((t, preview))
        });
        let png = self.png.and_then(|p| Some((png_size(&p)?, p)));
        let mut offers = Vec::new();
        let (kind, primary) = match (text, png.as_ref()) {
            (Some((t, preview)), _) => {
                offers.extend(crate::clipboard::text_offers(&t));
                (Kind::Text(preview), t)
            }
            (None, Some(((width, height), p))) => (
                Kind::Image {
                    width: *width,
                    height: *height,
                },
                p.clone(),
            ),
            (None, None) => return None,
        };
        if let Some((_, p)) = png {
            offers.push((PNG.to_string(), p));
        }
        offers.extend(self.extras);
        Some((kind, primary, offers))
    }
}

#[derive(Default)]
pub struct Recorder {
    pub history: History,
    /// The entry the seat's clipboard was recorded as, which outlives its owner.
    live: Option<u64>,
    capture: Option<Capture>,
    /// Bumped by every capture, so a watch or timer of an abandoned one
    /// that still fires finds nothing to act on.
    generation: u64,
}

/// A selection being read, one type at a time. The selection stays the
/// seat's while it lives: any change abandons it first.
struct Capture {
    generation: u64,
    steps: VecDeque<Step>,
    step: Option<Step>,
    watch: Option<RegistrationToken>,
    timer: Option<RegistrationToken>,
    buf: Vec<u8>,
    got: Got,
}

enum Read {
    More,
    Done,
    TooLarge,
}

impl State {
    pub fn record_selection(&mut self) {
        self.abandon_history_read();
        self.panels.history.live = None;
        if !self.config.clipboard.history {
            self.panels.history.history.clear();
            return;
        }
        if self.is_locked() {
            return;
        }
        if let Some(offers) = self.own_offers() {
            self.record(Got::from_offers(&offers));
            return;
        }
        let steps = plan(self.clip_offered());
        if steps.is_empty() {
            if !self.clip_offered().is_empty() {
                log::debug!(
                    "clipboard history: nothing to show in {:?}",
                    self.clip_offered()
                );
            }
            return;
        }
        let r = &mut self.panels.history;
        r.generation += 1;
        r.capture = Some(Capture {
            generation: r.generation,
            steps: steps.into(),
            step: None,
            watch: None,
            timer: None,
            buf: Vec::new(),
            got: Got::default(),
        });
        self.next_history_step();
    }

    /// Put entry `id` back on the clipboard, held by Reprise. False when the entry is gone.
    pub(crate) fn clip_offer_entry(&mut self, id: u64) -> bool {
        let Some(offers) = self
            .panels
            .history
            .history
            .get(id)
            .map(|e| e.offers.clone())
        else {
            return false;
        };
        self.clip_offer_each(offers);
        true
    }

    /// The client holding the clipboard destroyed its source without a new selection, which
    /// Smithay tells no handler: the copy recorded of it stays on the clipboard.
    pub(crate) fn clip_keep_orphaned(&mut self) {
        if self.clip.offered.is_empty() || data_device_selection_alive(&self.seat) {
            return;
        }
        self.clip.offered.clear();
        if let Some(id) = self.panels.history.live {
            self.clip_offer_entry(id);
        }
    }

    pub fn clear_history(&mut self) {
        self.abandon_history_read();
        self.panels.history.history.clear();
    }

    fn abandon_history_read(&mut self) {
        let Some(c) = self.panels.history.capture.take() else {
            return;
        };
        for t in [c.watch, c.timer].into_iter().flatten() {
            self.handle.remove(t);
        }
    }

    fn next_history_step(&mut self) {
        let Some(c) = self.panels.history.capture.as_mut() else {
            return;
        };
        if let Some(t) = c.timer.take() {
            self.handle.remove(t);
        }
        // The watch of the step before removed itself.
        c.buf = Vec::new();
        let Some(step) = c.steps.pop_front() else {
            let got = std::mem::take(&mut c.got);
            self.panels.history.capture = None;
            self.record(got);
            return;
        };
        let generation = c.generation;
        let (read, write) = match std::io::pipe() {
            Ok(p) => p,
            Err(e) => {
                log::warn!("clipboard history: pipe: {e}");
                self.abandon_history_read();
                return;
            }
        };
        if let Err(e) = self.request_clipboard(step.mime(), write.into()) {
            log::debug!("clipboard history: {e}");
            self.abandon_history_read();
            return;
        }
        let watch = self.handle.insert_source(
            Generic::new(read, Interest::READ, Mode::Level),
            move |_, pipe, state| Ok(state.history_readable(generation, &**pipe)),
        );
        let timer =
            self.handle
                .insert_source(Timer::from_duration(STEP_TIMEOUT), move |_, _, state| {
                    state.history_timed_out(generation);
                    TimeoutAction::Drop
                });
        let Some(c) = self.panels.history.capture.as_mut() else {
            return;
        };
        c.step = Some(step);
        match (watch, timer) {
            (Ok(w), Ok(t)) => {
                c.watch = Some(w);
                c.timer = Some(t);
            }
            (w, t) => {
                log::warn!("clipboard history: cannot watch the pipe");
                for t in [w.ok(), t.ok()].into_iter().flatten() {
                    self.handle.remove(t);
                }
                self.abandon_history_read();
            }
        }
    }

    fn history_timed_out(&mut self, generation: u64) {
        let Some(c) = self.panels.history.capture.as_mut() else {
            return;
        };
        if c.generation != generation {
            return;
        }
        log::debug!("clipboard history: the selection owner did not answer in time");
        // The timer drops itself.
        c.timer = None;
        self.abandon_history_read();
    }

    fn history_readable(&mut self, generation: u64, pipe: impl std::io::Read) -> PostAction {
        let Some(c) = self
            .panels
            .history
            .capture
            .as_mut()
            .filter(|c| c.generation == generation)
        else {
            return PostAction::Remove;
        };
        let Some(step) = c.step.clone() else {
            return PostAction::Remove;
        };
        match read_some(pipe, &mut c.buf, step.limit()) {
            Read::More => return PostAction::Continue,
            Read::TooLarge if matches!(step, Step::Extra(_)) => {
                c.watch = None;
                self.next_history_step();
                return PostAction::Remove;
            }
            Read::TooLarge => {
                log::debug!("clipboard history: the selection is too large to keep");
                c.watch = None;
                self.abandon_history_read();
                return PostAction::Remove;
            }
            Read::Done => {}
        }
        // This watch goes with the return; the next step makes its own.
        c.watch = None;
        let bytes = std::mem::take(&mut c.buf);
        match step {
            Step::Hint => {
                if bytes.trim_ascii() == b"secret" {
                    log::debug!("clipboard history: a secret selection; not kept");
                    self.abandon_history_read();
                    return PostAction::Remove;
                }
            }
            Step::Text(_) => c.got.text = Some(bytes.into()),
            Step::Png => c.got.png = Some(bytes.into()),
            Step::Extra(mime) => c.got.extras.push((mime, bytes.into())),
        }
        self.next_history_step();
        PostAction::Remove
    }

    fn record(&mut self, got: Got) {
        let Some((kind, primary, offers)) = got.into_entry() else {
            return;
        };
        let png = offers
            .iter()
            .find(|(m, _)| m == PNG)
            .map(|(_, b)| b.clone());
        let r = &mut self.panels.history;
        let Some(id) = r.history.push(kind, primary, offers) else {
            return;
        };
        r.live = Some(id);
        let needs_thumb = (r.history)
            .get(id)
            .is_some_and(|e| matches!(e.kind, Kind::Image { .. }) && e.thumb.is_none());
        let Some(png) = png.filter(|_| needs_thumb) else {
            return;
        };
        // A decode takes a while for a large image, so it runs off the
        // loop; the entry may be gone by the time it lands.
        self.spawn_job(move || {
            let thumb = thumbnail(&png);
            Box::new(move |state: &mut State| {
                let Some(thumb) = thumb else {
                    return;
                };
                if let Some(e) = state.panels.history.history.get_mut(id) {
                    e.thumb = Some(thumb.clone());
                    state.clip_thumb_ready(id, &thumb);
                }
            })
        });
    }
}

fn read_some(mut pipe: impl std::io::Read, buf: &mut Vec<u8>, limit: usize) -> Read {
    let mut chunk = [0u8; 16 * 1024];
    match pipe.read(&mut chunk) {
        Ok(0) => Read::Done,
        Ok(n) if buf.len() + n > limit => Read::TooLarge,
        Ok(n) => {
            buf.extend_from_slice(&chunk[..n]);
            Read::More
        }
        Err(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock
            ) =>
        {
            Read::More
        }
        Err(e) => {
            log::debug!("clipboard history: read: {e}");
            Read::Done
        }
    }
}

fn thumbnail(png: &[u8]) -> Option<Canvas> {
    let full = match reprisekit::icontheme::png_to_canvas(png, THUMB_MAX_SIDE) {
        Ok(c) => c,
        Err(e) => {
            log::debug!("clipboard history: no thumbnail: {e}");
            return None;
        }
    };
    let (w, h) = fit(full.width, full.height, THUMB);
    Some(full.resample_box(w, h))
}

pub fn fit(w: u32, h: u32, side: u32) -> (u32, u32) {
    if w <= side && h <= side {
        return (w.max(1), h.max(1));
    }
    if w >= h {
        (side, (h * side / w).max(1))
    } else {
        ((w * side / h).max(1), side)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> (Kind, Arc<[u8]>, Vec<Offer>) {
        let b: Arc<[u8]> = Arc::from(s.as_bytes());
        (
            Kind::Text(s.into()),
            b.clone(),
            crate::clipboard::text_offers(&b),
        )
    }

    fn push_text(h: &mut History, s: &str) -> Option<u64> {
        let (k, p, o) = text(s);
        h.push(k, p, o)
    }

    fn labels(h: &History) -> Vec<String> {
        h.entries().map(|e| e.kind.label()).collect()
    }

    fn png_offer(p: Arc<[u8]>) -> Vec<Offer> {
        vec![(PNG.to_string(), p)]
    }

    #[test]
    fn newest_first_and_a_repeat_moves_up_with_its_id() {
        let mut h = History::default();
        let a = push_text(&mut h, "a").unwrap();
        let b = push_text(&mut h, "b").unwrap();
        assert_ne!(a, b);
        assert_eq!(labels(&h), ["b", "a"]);
        assert_eq!(push_text(&mut h, "a"), Some(a));
        assert_eq!(labels(&h), ["a", "b"]);
    }

    #[test]
    fn keeps_twenty_five_entries() {
        let mut h = History::default();
        for i in 0..30 {
            push_text(&mut h, &i.to_string());
        }
        let l = labels(&h);
        assert_eq!(l.len(), MAX_ENTRIES);
        assert_eq!(l[0], "29");
        assert_eq!(l[24], "5");
    }

    #[test]
    fn drops_the_oldest_past_ten_megabytes_and_refuses_one_larger() {
        let mut h = History::default();
        let big = |fill: u8| -> (Kind, Arc<[u8]>, Vec<Offer>) {
            let p: Arc<[u8]> = vec![fill; 4 << 20].into();
            (
                Kind::Image {
                    width: 1,
                    height: fill as u32,
                },
                p.clone(),
                png_offer(p),
            )
        };
        for fill in 1..=3 {
            let (k, p, o) = big(fill);
            h.push(k, p, o).unwrap();
        }
        assert_eq!(labels(&h), ["Image, 1x3", "Image, 1x2"]);
        let p: Arc<[u8]> = vec![0u8; MAX_BYTES + 1].into();
        let refused = h.push(
            Kind::Image {
                width: 9,
                height: 9,
            },
            p.clone(),
            png_offer(p),
        );
        assert_eq!(refused, None);
        assert_eq!(labels(&h).len(), 2);
    }

    #[test]
    fn shared_text_buffers_count_once() {
        let mut h = History::default();
        let id = push_text(&mut h, "hello").unwrap();
        let e = h.get(id).unwrap();
        assert_eq!(e.offers.len(), TEXT_MIMES.len());
        assert_eq!(e.size(), 5);
    }

    #[test]
    fn remove_and_clear() {
        let mut h = History::default();
        let a = push_text(&mut h, "a").unwrap();
        push_text(&mut h, "b");
        h.remove(a);
        assert_eq!(labels(&h), ["b"]);
        h.clear();
        assert!(labels(&h).is_empty());
    }

    #[test]
    fn preview_is_one_line_and_blank_text_has_none() {
        assert_eq!(
            preview("  one\n\ttwo  three \n").as_deref(),
            Some("one two three")
        );
        assert_eq!(preview(" \n\t "), None);
        assert_eq!(preview(&"x".repeat(500)).unwrap().chars().count(), 120);
    }

    #[test]
    fn png_size_reads_the_header() {
        let mut c = Canvas::new(7, 3);
        c.fill_rect(c.bounds(), reprisekit::theme::Color::rgb(1, 2, 3));
        let png = reprisekit::raster::to_png(&c);
        assert_eq!(png_size(&png), Some((7, 3)));
        assert_eq!(png_size(b"hello, this is not a png at all"), None);
        let thumb = thumbnail(&png).unwrap();
        assert_eq!((thumb.width, thumb.height), (7, 3));
    }

    #[test]
    fn fit_keeps_the_aspect_and_never_grows() {
        assert_eq!(fit(1920, 1080, 32), (32, 18));
        assert_eq!(fit(100, 400, 32), (8, 32));
        assert_eq!(fit(10, 5, 32), (10, 5));
        assert_eq!(fit(4000, 1, 32), (32, 1));
    }

    fn strs(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_plan_reads_the_hint_first_then_text_png_and_small_extras() {
        let offered = strs(&[
            "text/html",
            "UTF8_STRING",
            "text/plain",
            "image/png",
            "x-kde-passwordManagerHint",
            "TARGETS",
            "text/plain;charset=utf-8",
            "text/html",
        ]);
        assert_eq!(
            plan(&offered),
            vec![
                Step::Hint,
                Step::Text("text/plain;charset=utf-8".into()),
                Step::Png,
                Step::Extra("text/html".into()),
            ]
        );
    }

    #[test]
    fn nothing_is_read_from_a_selection_with_no_text_or_png() {
        assert!(plan(&strs(&["text/uri-list", "x-special/gnome-copied-files"])).is_empty());
        assert!(plan(&strs(&["x-kde-passwordManagerHint"])).is_empty());
        assert_eq!(plan(&strs(&["image/png"])), vec![Step::Png]);
    }

    #[test]
    fn extras_stop_at_eight() {
        let many: Vec<String> = (0..20).map(|i| format!("application/x-{i}")).collect();
        let mut offered = many.clone();
        offered.push("TEXT".into());
        let steps = plan(&offered);
        assert_eq!(steps.len(), 1 + MAX_EXTRAS);
        assert_eq!(steps[0], Step::Text("TEXT".into()));
    }

    #[test]
    fn text_wins_over_a_png_and_both_are_offered_back() {
        let t: Arc<[u8]> = Arc::from(&b"cells"[..]);
        let mut c = Canvas::new(2, 2);
        c.fill_rect(c.bounds(), reprisekit::theme::Color::rgb(9, 9, 9));
        let p: Arc<[u8]> = reprisekit::raster::to_png(&c).into();
        let got = Got {
            text: Some(t),
            png: Some(p),
            extras: vec![("text/html".into(), Arc::from(&b"<b>cells</b>"[..]))],
        };
        let (kind, primary, offers) = got.into_entry().unwrap();
        assert_eq!(kind, Kind::Text("cells".into()));
        assert_eq!(&primary[..], b"cells");
        let mimes: Vec<&str> = offers.iter().map(|(m, _)| m.as_str()).collect();
        assert_eq!(
            mimes,
            [
                "text/plain;charset=utf-8",
                "text/plain",
                "UTF8_STRING",
                "TEXT",
                "image/png",
                "text/html"
            ]
        );
    }

    #[test]
    fn an_image_entry_needs_a_real_png() {
        let got = Got {
            text: None,
            png: Some(Arc::from(&b"not a png"[..])),
            extras: Vec::new(),
        };
        assert!(got.into_entry().is_none());
        let blank = Got {
            text: Some(Arc::from(&b"   "[..])),
            ..Got::default()
        };
        assert!(blank.into_entry().is_none());
    }

    #[test]
    fn a_selection_of_reprises_is_sorted_by_type_and_drops_the_hint() {
        let t: Arc<[u8]> = Arc::from(&b"hi"[..]);
        let mut offers = crate::clipboard::text_offers(&t);
        offers.push((HINT.into(), Arc::from(&b"x"[..])));
        offers.push(("text/html".into(), Arc::from(&b"<i>hi</i>"[..])));
        let got = Got::from_offers(&offers);
        assert_eq!(got.text.as_deref(), Some(&b"hi"[..]));
        assert!(got.png.is_none());
        assert_eq!(got.extras.len(), 1);
    }

    #[test]
    fn a_read_stops_at_the_steps_limit() {
        let mut buf = Vec::new();
        assert!(matches!(
            read_some(&b"0123456789"[..], &mut buf, 64),
            Read::More
        ));
        assert_eq!(buf, b"0123456789");
        assert!(matches!(read_some(&b""[..], &mut buf, 64), Read::Done));
        let mut buf = Vec::new();
        assert!(matches!(
            read_some(&[0u8; 65][..], &mut buf, 64),
            Read::TooLarge
        ));
    }
}
