//! The Properties sheet's Checksums tab: hashing on a thread of its own, its progress and result
//! landing at a wake, Copy, and the Compare field.

use crate::Explorer;
use crate::props_dialog::PROPS;
use repriseapp::{Ctx, Reply};
use reprisefs::checksum::{Meter, Sums, compare, sums_of};
use reprisekit::controls::filesheet::{self, SumsState};
use reprisekit::controls::form;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

/// Files up to this size are hashed as soon as the tab opens; larger ones wait for Calculate.
pub(crate) const AUTO_LIMIT: u64 = 100 << 20;
const PROGRESS_EVERY: Duration = Duration::from_millis(100);

type Answer = std::io::Result<Option<Sums>>;

/// The sheet's hashing. Dropping it (the sheet closing) cancels the read; a read stuck on a dead
/// mount stays on its own thread, which nothing waits for.
#[derive(Default)]
pub(crate) struct Checksums {
    meter: Arc<Meter>,
    answer: Option<Receiver<Answer>>,
    pub(crate) sums: Option<Sums>,
    started: bool,
    shown_percent: Option<u8>,
}

impl Drop for Checksums {
    fn drop(&mut self) {
        self.meter.cancel.store(true, Ordering::Relaxed);
    }
}

fn percent(done: u64, total: u64) -> u8 {
    match total {
        0 => 0,
        t => (done.min(t) * 100 / t) as u8,
    }
}

impl Explorer {
    /// The Checksums page's buttons and Compare field; `None` for any other control.
    pub(crate) fn checksums_event(&mut self, ev: form::Event, ctx: &mut Ctx) -> Option<Reply> {
        let p = self.props.as_mut()?;
        match ev {
            form::Event::Pressed(filesheet::SUMS_CALCULATE) => {
                self.start_checksums(ctx);
                Some(Reply::redraw())
            }
            form::Event::Pressed(id @ (filesheet::SUMS_COPY_SHA256 | filesheet::SUMS_COPY_MD5)) => {
                let sums = p.checksums.sums.as_ref()?;
                let hash = match id {
                    filesheet::SUMS_COPY_SHA256 => &sums.sha256,
                    _ => &sums.md5,
                };
                Some(Reply {
                    copy: Some(hash.clone()),
                    ..Reply::none()
                })
            }
            form::Event::Changed(filesheet::SUMS_COMPARE) => {
                show_verdict(p);
                Some(Reply::redraw())
            }
            _ => None,
        }
    }

    /// The Checksums tab came up: a file up to [`AUTO_LIMIT`] starts hashing unasked, once.
    pub(crate) fn checksums_opened(&mut self, ctx: &mut Ctx) -> Reply {
        let Some(p) = self.props.as_ref() else {
            return Reply::none();
        };
        let open = p.sheet.open().item(filesheet::SUMS_CALCULATE).is_some();
        if !open || p.checksums.started || p.info.size > AUTO_LIMIT {
            return Reply::none();
        }
        self.start_checksums(ctx);
        Reply::redraw()
    }

    fn start_checksums(&mut self, ctx: &mut Ctx) {
        let Some(p) = self.props.as_mut() else {
            return;
        };
        if p.checksums.answer.is_some() || p.checksums.sums.is_some() {
            return;
        }
        let meter = Arc::new(Meter::default());
        let (tx, rx) = std::sync::mpsc::channel();
        let (path, thread_meter, wake) = (p.info.path.clone(), meter.clone(), ctx.wake.clone());
        let spawned = std::thread::Builder::new()
            .name("reprise-explorer-sums".into())
            .spawn(move || {
                let mut last = Instant::now();
                let answer = sums_of(&path, &thread_meter, || {
                    if last.elapsed() >= PROGRESS_EVERY {
                        last = Instant::now();
                        wake.wake();
                    }
                });
                if tx.send(answer).is_ok() {
                    wake.wake();
                }
            });
        let state = match spawned {
            Ok(_) => {
                p.checksums = Checksums {
                    meter,
                    answer: Some(rx),
                    sums: None,
                    started: true,
                    shown_percent: Some(0),
                };
                SumsState::Working(0)
            }
            Err(e) => {
                log::warn!("explorer: cannot start hashing ({e})");
                p.checksums.started = true;
                SumsState::Failed(format!("Cannot calculate the checksums: {e}"))
            }
        };
        if let Some(page) = filesheet::checksums_of(&mut p.sheet.pages) {
            filesheet::show_sums(page, &state);
        }
    }

    /// At a wake: the hashing's progress, or its answer, onto the page.
    pub(crate) fn take_checksums(&mut self) -> Option<Reply> {
        let p = self.props.as_mut()?;
        let c = &mut p.checksums;
        let rx = c.answer.as_ref()?;
        let state = match rx.try_recv() {
            Ok(Ok(Some(sums))) => {
                let state = SumsState::Done {
                    sha256: sums.sha256.clone(),
                    md5: sums.md5.clone(),
                };
                c.sums = Some(sums);
                c.answer = None;
                state
            }
            Ok(Ok(None)) | Err(TryRecvError::Disconnected) => {
                c.answer = None;
                SumsState::Idle
            }
            Ok(Err(e)) => {
                c.answer = None;
                SumsState::Failed(format!("Cannot read the file: {e}"))
            }
            Err(TryRecvError::Empty) => {
                let now = percent(c.meter.done.load(Ordering::Relaxed), p.info.size);
                if c.shown_percent == Some(now) {
                    return None;
                }
                c.shown_percent = Some(now);
                SumsState::Working(now)
            }
        };
        if let Some(page) = filesheet::checksums_of(&mut p.sheet.pages) {
            filesheet::show_sums(page, &state);
        }
        show_verdict(p);
        Some(Reply {
            redraw_wins: vec![PROPS],
            ..Reply::none()
        })
    }
}

fn show_verdict(p: &mut crate::props_dialog::Props) {
    let Some(page) = filesheet::checksums_of(&mut p.sheet.pages) else {
        return;
    };
    let text = match &p.checksums.sums {
        Some(sums) => compare(filesheet::compare_text(page), sums).text(),
        None => String::new(),
    };
    filesheet::show_verdict(page, &text);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::place::Place;
    use reprisekit::controls::propsheet;

    fn sheet_on(path: &std::path::Path, ctx: &mut Ctx) -> Explorer {
        let dir = path.parent().unwrap().to_path_buf();
        let mut ex = crate::tests::test_explorer(Place::Folder(dir), ctx);
        let name = path.file_name().unwrap().to_str().unwrap().to_string();
        ex.open_properties(path, &name, &name, ctx);
        let p = ex.props.as_mut().expect("the sheet");
        assert_eq!(p.sheet.tabs.labels, ["General", "Summary", "Checksums"]);
        p.sheet.open_tab(2);
        ex
    }

    fn page_event(ex: &mut Explorer, ev: form::Event, ctx: &mut Ctx) -> Reply {
        ex.props_outcome(propsheet::Event::Page(2, ev), ctx)
    }

    fn wait_for_sums(ex: &mut Explorer) {
        let end = Instant::now() + Duration::from_secs(10);
        while ex.props.as_ref().unwrap().checksums.sums.is_none() {
            assert!(Instant::now() < end, "the sums land");
            ex.take_checksums();
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn a_small_file_hashes_when_its_tab_opens_and_copies_and_compares() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("abc.txt");
        std::fs::write(&f, "abc").unwrap();
        let mut ctx = crate::tests::test_ctx();
        let mut ex = sheet_on(&f, &mut ctx);
        ex.checksums_opened(&mut ctx);
        wait_for_sums(&mut ex);
        let copied = page_event(
            &mut ex,
            form::Event::Pressed(filesheet::SUMS_COPY_MD5),
            &mut ctx,
        );
        assert_eq!(
            copied.copy.as_deref(),
            Some("900150983cd24fb0d6963f7d28e17f72")
        );
        let p = ex.props.as_mut().unwrap();
        let page = filesheet::checksums_of(&mut p.sheet.pages).unwrap();
        page.focus_id(filesheet::SUMS_COMPARE);
        for c in " 900150983CD24FB0D6963F7D28E17F72 ".chars() {
            page.key(form::Key::Char(c));
        }
        page_event(
            &mut ex,
            form::Event::Changed(filesheet::SUMS_COMPARE),
            &mut ctx,
        );
        let p = ex.props.as_mut().unwrap();
        let page = filesheet::checksums_of(&mut p.sheet.pages).unwrap();
        let verdict = match &page.item("sums_verdict").unwrap().control {
            form::Control::Label { text, .. } => text.clone(),
            other => panic!("{other:?}"),
        };
        assert_eq!(verdict, "Matches MD5");
    }

    #[test]
    fn a_file_past_the_limit_waits_for_calculate() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("abc.txt");
        std::fs::write(&f, "abc").unwrap();
        let mut ctx = crate::tests::test_ctx();
        let mut ex = sheet_on(&f, &mut ctx);
        ex.props.as_mut().unwrap().info.size = AUTO_LIMIT + 1;
        ex.checksums_opened(&mut ctx);
        assert!(!ex.props.as_ref().unwrap().checksums.started);
        page_event(
            &mut ex,
            form::Event::Pressed(filesheet::SUMS_CALCULATE),
            &mut ctx,
        );
        wait_for_sums(&mut ex);
    }

    /// A read that never answers, as on a dead mount: the sheet still closes at once, and the
    /// read is told to stop.
    #[test]
    fn closing_the_sheet_cancels_a_read_that_hangs() {
        let dir = tempfile::tempdir().unwrap();
        let fifo = dir.path().join("pipe");
        let made = std::process::Command::new("mkfifo").arg(&fifo).status();
        if !made.is_ok_and(|s| s.success()) {
            eprintln!("no mkfifo; skipped");
            return;
        }
        let mut ctx = crate::tests::test_ctx();
        let mut ex = sheet_on(&fifo, &mut ctx);
        page_event(
            &mut ex,
            form::Event::Pressed(filesheet::SUMS_CALCULATE),
            &mut ctx,
        );
        let meter = ex.props.as_ref().unwrap().checksums.meter.clone();
        assert!(ex.take_checksums().is_none(), "still waiting on the open");
        ex.props_event(repriseapp::Event::CloseRequest, &mut ctx);
        assert!(ex.props.is_none());
        assert!(meter.cancel.load(Ordering::Relaxed));
        drop(std::fs::OpenOptions::new().write(true).open(&fifo));
        let end = Instant::now() + Duration::from_secs(10);
        while Arc::strong_count(&meter) > 1 {
            assert!(Instant::now() < end, "the freed read stops");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(meter.done.load(Ordering::Relaxed), 0, "nothing was read");
    }

    #[test]
    fn percent_is_of_the_files_size() {
        assert_eq!(percent(0, 200), 0);
        assert_eq!(percent(50, 200), 25);
        assert_eq!(percent(300, 200), 100, "a file that grew reads as done");
        assert_eq!(percent(0, 0), 0, "a size of 0 (a pipe) shows no progress");
    }
}
