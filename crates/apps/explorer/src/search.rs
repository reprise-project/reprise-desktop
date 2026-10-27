//! The toolbar's Search field: [`reprisefs::search`] rooted at the folder shown runs on a listing
//! thread, and its results take the folder's place in the view until Escape or a navigation.

use crate::place::{Listed, Place};
use crate::{Explorer, Focus, Read};
use repriseapp::listicons::ReadIcons;
use repriseapp::{Ctx, Reply};
use reprisefs::search::Query;
use reprisekit::controls::shellview;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// A folder change re-runs the search no sooner than this after the last run started.
pub(crate) const RERUN_AFTER: Duration = Duration::from_secs(5);

/// What was searched for and where.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Asked {
    pub place: Place,
    pub text: String,
}

/// A search under way; dropping it stops the tool.
struct Running {
    serial: u64,
    asked: Asked,
    cancel: Arc<AtomicBool>,
}

impl Drop for Running {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// A search just started, for its thread.
pub(crate) struct Started {
    pub serial: u64,
    pub query: Query,
    pub cancel: Arc<AtomicBool>,
}

/// The search's state: the run under way, and what the view's results answer.
#[derive(Default)]
pub(crate) struct Search {
    serial: u64,
    running: Option<Running>,
    shown: Option<Asked>,
    /// The folder changed since the last run started, which searches again once it lands and
    /// [`RERUN_AFTER`] has passed.
    stale: bool,
    last_start: Option<Instant>,
    /// The re-run deadline a wake was asked for.
    armed: Option<Instant>,
}

/// What a held folder change asks for ([`Search::take_stale`]).
#[derive(Debug, PartialEq)]
pub(crate) enum Stale {
    None,
    Now,
    /// A wake at this instant, asked once per deadline.
    WakeAt(Instant),
}

impl Search {
    /// Starts a search for `text` under `place`'s folder, stopping one under way. `None` for blank
    /// text or a place with no folder (Computer, the Recycle Bin).
    pub fn start(&mut self, place: &Place, text: &str, hidden: bool) -> Option<Started> {
        let text = text.trim();
        let root = place.path().filter(|_| *place != Place::RecycleBin)?;
        if text.is_empty() {
            return None;
        }
        self.serial += 1;
        self.stale = false;
        self.last_start = Some(Instant::now());
        let mut query = Query::folder(text, &root);
        query.filter.hidden = hidden;
        let cancel = Arc::new(AtomicBool::new(false));
        self.running = Some(Running {
            serial: self.serial,
            asked: Asked {
                place: place.clone(),
                text: text.to_string(),
            },
            cancel: cancel.clone(),
        });
        Some(Started {
            serial: self.serial,
            query,
            cancel,
        })
    }

    /// Stops the run under way and drops the results; true when either was there.
    pub fn cancel(&mut self) -> bool {
        let had = self.running.is_some() || self.shown.is_some();
        self.running = None;
        self.shown = None;
        self.stale = false;
        had
    }

    /// The run `serial` answered: true when it is the latest, whose results the view now shows.
    pub fn landed(&mut self, serial: u64) -> bool {
        match self.running.take_if(|r| r.serial == serial) {
            Some(r) => {
                self.shown = Some(r.asked.clone());
                true
            }
            None => false,
        }
    }

    /// The folder watch saw a change: true when a run is under way or its results are shown,
    /// which keeps the change for [`Search::take_stale`] instead of restarting the walk.
    pub fn hold_change(&mut self) -> bool {
        let held = self.running.is_some() || self.shown.is_some();
        self.stale |= held;
        held
    }

    /// Whether a held change searches again: not while a run is under way, and not before
    /// [`RERUN_AFTER`] from the last run's start.
    pub fn take_stale(&mut self, now: Instant) -> Stale {
        if !self.stale || self.running.is_some() {
            return Stale::None;
        }
        let due = self.last_start.map_or(now, |t| t + RERUN_AFTER);
        if now >= due {
            self.stale = false;
            return Stale::Now;
        }
        match self.armed.replace(due) {
            Some(t) if t == due => Stale::None,
            _ => Stale::WakeAt(due),
        }
    }

    /// What the view's results answer.
    pub fn shown(&self) -> Option<&Asked> {
        self.shown.as_ref()
    }

    pub fn running(&self) -> bool {
        self.running.is_some()
    }

    /// The window's title while results are shown.
    pub fn caption(&self) -> Option<String> {
        let a = self.shown.as_ref()?;
        Some(format!("Search results in {}", a.place.title()))
    }
}

/// A search's answer, read on its thread.
pub(crate) struct Found {
    serial: u64,
    read: Read,
}

pub(crate) type Runner = fn(&Query, &AtomicBool) -> Vec<PathBuf>;

impl Explorer {
    /// Ctrl+F and F3: the keyboard goes to the Search field, its text selected.
    pub(crate) fn search_focus(&mut self) -> Reply {
        self.focus = Focus::Search;
        self.search_field.focus();
        Reply::redraw()
    }

    /// Enter in the Search field: searches the folder shown for the field's text; an answer
    /// later than the listing wait lands on a wake ([`Explorer::take_search`]).
    pub(crate) fn run_search(&mut self, ctx: &mut Ctx) -> Reply {
        let text = self.search_field.text().to_string();
        let reply = self.search_for(&text, ctx);
        if self.search.running() || self.search.shown().is_some() {
            self.focus = Focus::View;
        }
        reply
    }

    fn search_for(&mut self, text: &str, ctx: &mut Ctx) -> Reply {
        let Some(started) = self
            .search
            .start(&self.place, text, self.settings.show_hidden)
        else {
            return Reply::none();
        };
        if let Some(r) = self.reading.take() {
            self.leave(r);
        }
        let run = self.search_run;
        let globs = self.globs.clone();
        let place = self.place.clone();
        let scale = ctx.icons.scale();
        let list = shellview::list_icon_px(ctx.theme.text);
        let read = move || {
            let Started {
                serial,
                query,
                cancel,
            } = started;
            let items: Vec<_> = run(&query, &cancel)
                .iter()
                .map(|p| reprisefs::listing::item_at(p, &globs))
                .collect();
            let icons = ReadIcons::read(&items, reprisekit::icontheme::listing_px(list), scale);
            Found {
                serial,
                read: Read {
                    place,
                    listed: Listed {
                        items,
                        groups: Vec::new(),
                        drives: Vec::new(),
                    },
                    icons,
                    list,
                    tree: Vec::new(),
                },
            }
        };
        match self.read_job(ctx, read) {
            Ok(found) => self.show_found(found, ctx),
            Err(job) => {
                self.search_job = Some(job);
                Reply::redraw()
            }
        }
    }

    /// Shows a search that answered late ([`Explorer::run_search`]).
    pub(crate) fn take_search(&mut self, ctx: &mut Ctx) -> Option<Reply> {
        let found = self.search_job.as_ref()?.take()?;
        self.search_job = None;
        Some(self.show_found(found, ctx))
    }

    /// The results of the run `found` answers, unless a newer run, Escape or a navigation came
    /// since. Results for the text already shown (a refresh) keep the selection and scroll.
    fn show_found(&mut self, found: Found, ctx: &mut Ctx) -> Reply {
        let again = self.search.shown().cloned();
        if !self.search.landed(found.serial) || found.read.place != self.place {
            return Reply::none();
        }
        let reply = match again.as_ref() == self.search.shown() {
            true => self.keeping_view(ctx, |ex, ctx| ex.show_read(found.read, ctx)),
            false => self.show_read(found.read, ctx),
        };
        reply.and(self.rerun_stale(ctx).unwrap_or_default())
    }

    /// Searches again for a held folder change once it is due, or asks for a wake when it will be.
    pub(crate) fn rerun_stale(&mut self, ctx: &mut Ctx) -> Option<Reply> {
        match self.search.take_stale(Instant::now()) {
            Stale::None => None,
            Stale::Now => self.search_again(ctx),
            Stale::WakeAt(at) => {
                let wake = ctx.wake.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(at.saturating_duration_since(Instant::now()));
                    wake.wake();
                });
                None
            }
        }
    }

    /// Escape: a search under way stops, and results shown give way to the folder's listing.
    /// `None` when there was neither.
    pub(crate) fn cancel_search(&mut self, ctx: &mut Ctx) -> Option<Reply> {
        let shown = self.search.shown().is_some();
        if !self.search.cancel() {
            return None;
        }
        self.search_job = None;
        Some(match shown {
            true => self.refresh(ctx),
            false => Reply::redraw(),
        })
    }

    /// A refresh while results are shown searches again for the same text.
    pub(crate) fn search_again(&mut self, ctx: &mut Ctx) -> Option<Reply> {
        let text = self.search.shown()?.text.clone();
        Some(self.search_for(&text, ctx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(p: &str) -> Place {
        Place::Folder(PathBuf::from(p))
    }

    #[test]
    fn a_search_needs_text_and_a_folder() {
        let mut s = Search::default();
        assert!(s.start(&folder("/data"), "  ", false).is_none());
        assert!(s.start(&Place::Computer, "notes", false).is_none());
        assert!(s.start(&Place::RecycleBin, "notes", false).is_none());
        assert!(!s.running());
        let run = s.start(&folder("/data"), " notes ", true).expect("a run");
        assert_eq!(run.query.text, "notes", "the text is trimmed");
        assert_eq!(run.query.root, PathBuf::from("/data"));
        assert_eq!(run.query.depth, None, "every level under the folder");
        assert!(run.query.filter.hidden);
        assert!(s.running());
    }

    #[test]
    fn a_new_search_stops_the_one_under_way_whose_answer_is_then_dropped() {
        let mut s = Search::default();
        let first = s.start(&folder("/data"), "a", false).expect("first");
        let second = s.start(&folder("/data"), "ab", false).expect("second");
        assert!(first.cancel.load(Ordering::Relaxed), "the first run stops");
        assert!(!second.cancel.load(Ordering::Relaxed));
        assert!(!s.landed(first.serial), "its answer is dropped");
        assert!(s.landed(second.serial));
        assert_eq!(
            s.shown(),
            Some(&Asked {
                place: folder("/data"),
                text: "ab".into()
            })
        );
        assert!(!s.running());
        assert_eq!(s.caption().as_deref(), Some("Search results in data"));
    }

    #[test]
    fn a_change_during_a_run_is_held_and_searches_once_more_after_it_lands() {
        let mut s = Search::default();
        assert!(!s.hold_change(), "no run: the change refreshes at once");
        let run = s.start(&folder("/data"), "a", false).expect("a run");
        let later = Instant::now() + RERUN_AFTER;
        assert!(s.hold_change());
        assert!(s.hold_change(), "every change during the run is held");
        assert_eq!(
            s.take_stale(later),
            Stale::None,
            "not while the run is under way"
        );
        assert!(s.landed(run.serial));
        assert_eq!(
            s.take_stale(later),
            Stale::Now,
            "the landed run searches once more"
        );
        assert_eq!(s.take_stale(later), Stale::None);
        let run = s.start(&folder("/data"), "a", false).expect("a run");
        assert!(s.hold_change());
        let fresh = s.start(&folder("/data"), "a", false).expect("a new run");
        assert_eq!(
            s.take_stale(later + RERUN_AFTER),
            Stale::None,
            "a new run already sees the change"
        );
        assert!(!s.landed(run.serial) && s.landed(fresh.serial));
    }

    #[test]
    fn a_held_change_searches_again_five_seconds_after_the_last_run_started() {
        let mut s = Search::default();
        let before = Instant::now();
        let run = s.start(&folder("/data"), "a", false).expect("a run");
        assert!(s.hold_change());
        assert!(s.landed(run.serial));
        let Stale::WakeAt(at) = s.take_stale(Instant::now()) else {
            panic!("a re-run right after a landing waits for a wake");
        };
        assert!(at >= before + RERUN_AFTER && at <= Instant::now() + RERUN_AFTER);
        assert_eq!(
            s.take_stale(Instant::now()),
            Stale::None,
            "one wake per deadline"
        );
        assert!(s.hold_change(), "results shown: the change is held too");
        assert_eq!(s.take_stale(at), Stale::Now, "due once the 5 s are up");
        assert_eq!(s.take_stale(at), Stale::None);
    }

    #[test]
    fn cancel_stops_the_run_and_drops_the_results() {
        let mut s = Search::default();
        assert!(!s.cancel(), "nothing to cancel");
        let run = s.start(&folder("/data"), "a", false).expect("a run");
        assert!(s.cancel());
        assert!(run.cancel.load(Ordering::Relaxed));
        assert!(!s.landed(run.serial), "a cancelled run never shows");
        let run = s.start(&folder("/data"), "a", false).expect("a run");
        assert!(s.landed(run.serial));
        assert!(s.cancel());
        assert_eq!(s.shown(), None);
        assert_eq!(s.caption(), None);
    }
}
