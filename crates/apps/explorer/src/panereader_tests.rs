use super::*;
use std::sync::RwLock;
use std::sync::atomic::AtomicUsize;

/// [`Parts::installed`] with every fetch held until `gate`'s write lock drops, no thumbnail cache,
/// and `sampled` set when the font sampler runs.
fn held(gate: &Arc<RwLock<()>>, sampled: &Arc<AtomicBool>) -> Parts {
    let installed = Parts::installed();
    let (real, gate, flag) = (installed.fetch.clone(), gate.clone(), sampled.clone());
    Parts {
        fetch: Arc::new(move |p, cap, opened| {
            drop(gate.read());
            real(p, cap, opened)
        }),
        sample: Arc::new(move |_, _, _| {
            flag.store(true, Ordering::SeqCst);
            None
        }),
        cache: false,
        ..installed
    }
}

/// A guard giving up after 300 ms, counting nothing against other calls.
fn short_guard() -> Guard {
    Arc::new(|_, call| {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let _ = tx.send(call());
        });
        rx.recv_timeout(Duration::from_millis(300))
            .unwrap_or_else(|_| Err(io::ErrorKind::TimedOut.into()))
    })
}

fn request(path: &Path, route: Route) -> Request {
    Request {
        key: (path.to_path_buf(), 0, None),
        route,
        max: (64, 64),
        scale: 1,
    }
}

fn text_of(l: &Loaded) -> Option<&str> {
    match l {
        Loaded::Text(t) => Some(t),
        _ => None,
    }
}

#[test]
fn every_route_gives_up_at_the_one_deadline_and_its_folder_then_answers_at_once() {
    let dir = tempfile::tempdir().expect("a temp dir");
    for (name, route) in [
        ("a.txt", Route::Text),
        ("a.png", Route::Picture),
        ("a.ttf", Route::Font),
        ("a.dat", Route::Other),
    ] {
        let path = dir.path().join(name);
        std::fs::write(&path, "bytes").expect("write");
        let gate = Arc::new(RwLock::new(()));
        let sampled = Arc::new(AtomicBool::new(false));
        let closed = gate.write().expect("lock");
        let parts = held(&gate, &sampled);
        let mut marks = Marks::default();
        let started = Instant::now();
        let got = read(&parts, &mut marks, &request(&path, route));
        let took = started.elapsed();
        assert!(matches!(got, Loaded::Nothing), "{route:?}: no preview");
        assert!(
            took >= Duration::from_millis(2500) && took < Duration::from_secs(6),
            "{route:?}: gave up after {took:?}"
        );
        let again = Instant::now();
        let got = read(&parts, &mut marks, &request(&path, route));
        assert!(matches!(got, Loaded::Nothing));
        assert!(
            again.elapsed() < Duration::from_millis(100),
            "{route:?}: the marked folder answers at once"
        );
        assert!(
            !sampled.load(Ordering::SeqCst),
            "{route:?}: the sampler never ran"
        );
        drop(closed);
    }
}

#[test]
fn a_stalled_open_through_a_link_marks_only_the_links_folder() {
    let root = tempfile::tempdir().expect("a temp dir");
    let (links, other) = (root.path().join("links"), root.path().join("other"));
    std::fs::create_dir(&links).expect("mkdir");
    std::fs::create_dir(&other).expect("mkdir");
    std::fs::write(other.join("b.txt"), "beside\n").expect("write");
    std::os::unix::fs::symlink("/nowhere/dead/file.txt", links.join("dead.txt")).expect("link");
    let real = Parts::installed().fetch;
    let parts = Parts {
        fetch: Arc::new(move |p, cap, opened| {
            if p.ends_with("dead.txt") {
                std::thread::sleep(Duration::from_secs(5));
            }
            real(p, cap, opened)
        }),
        guard: short_guard(),
        cache: false,
        ..Parts::installed()
    };
    let mut marks = Marks::default();
    read(
        &parts,
        &mut marks,
        &request(&links.join("dead.txt"), Route::Text),
    );
    assert!(
        marks.holds(&links, Instant::now()),
        "the link's folder is marked"
    );
    assert!(
        !marks.holds(root.path(), Instant::now()),
        "not the mount it lives on"
    );
    let got = read(
        &parts,
        &mut marks,
        &request(&other.join("b.txt"), Route::Text),
    );
    assert_eq!(text_of(&got), Some("beside\n"), "a folder beside it reads");
}

#[test]
fn a_read_slow_after_its_open_marks_nothing() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("a.txt");
    std::fs::write(&path, "slow\n").expect("write");
    let calls = Arc::new(AtomicUsize::new(0));
    let (real, n) = (Parts::installed().fetch, calls.clone());
    let parts = Parts {
        fetch: Arc::new(move |p, cap, opened| {
            if n.fetch_add(1, Ordering::SeqCst) == 0 {
                opened.store(true, Ordering::SeqCst);
                std::thread::sleep(Duration::from_secs(5));
            }
            real(p, cap, opened)
        }),
        guard: short_guard(),
        cache: false,
        ..Parts::installed()
    };
    let mut marks = Marks::default();
    let first = read(&parts, &mut marks, &request(&path, Route::Text));
    assert!(matches!(first, Loaded::Nothing), "the slow read gave up");
    let again = read(&parts, &mut marks, &request(&path, Route::Text));
    assert_eq!(
        text_of(&again),
        Some("slow\n"),
        "read again, nothing marked"
    );
}

#[test]
fn a_call_refused_for_a_full_budget_marks_nothing() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("a.txt");
    std::fs::write(&path, "fine\n").expect("write");
    let refused = Arc::new(AtomicBool::new(true));
    let (installed, refuse) = (Parts::installed(), refused.clone());
    let guard = installed.guard.clone();
    let parts = Parts {
        guard: Arc::new(move |p, call| {
            if refuse.swap(false, Ordering::SeqCst) {
                let full = repriseapp::thumbs::BudgetFull(p.to_path_buf());
                return Err(io::Error::new(io::ErrorKind::TimedOut, full));
            }
            guard(p, call)
        }),
        cache: false,
        ..installed
    };
    let mut marks = Marks::default();
    let first = read(&parts, &mut marks, &request(&path, Route::Text));
    assert!(matches!(first, Loaded::Nothing), "no preview while refused");
    let again = read(&parts, &mut marks, &request(&path, Route::Text));
    assert_eq!(text_of(&again), Some("fine\n"), "nothing marked");
}

#[test]
fn a_mark_expires() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("a.txt");
    std::fs::write(&path, "back\n").expect("write");
    let parts = Parts {
        cache: false,
        ..Parts::installed()
    };
    let mut marks = Marks::default();
    let now = Instant::now();
    marks.mark(dir.path().to_path_buf(), now);
    let got = read(&parts, &mut marks, &request(&path, Route::Text));
    assert!(matches!(got, Loaded::Nothing), "marked: no read");
    let then = now
        .checked_sub(MARK_FOR + Duration::from_secs(1))
        .expect("an earlier instant");
    marks.mark(dir.path().to_path_buf(), then);
    let got = read(&parts, &mut marks, &request(&path, Route::Text));
    assert_eq!(text_of(&got), Some("back\n"), "the mark lapsed");
}

#[test]
fn a_route_that_panics_answers_no_preview_and_the_reader_goes_on() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let (font, text) = (dir.path().join("a.ttf"), dir.path().join("a.txt"));
    std::fs::write(&font, "face").expect("write");
    std::fs::write(&text, "after\n").expect("write");
    let parts = Parts {
        sample: Arc::new(|_, _, _| panic!("a sampler bug")),
        cache: false,
        ..Parts::installed()
    };
    let r = Reader::spawn(parts, Waker::noop());
    let answer = |r: &Reader| {
        let end = Instant::now() + Duration::from_secs(10);
        loop {
            match r.take() {
                Taken::Answer(_, l) => return l,
                Taken::Gone => panic!("the reader's thread died"),
                Taken::Waiting => {}
            }
            assert!(Instant::now() < end, "answered");
            std::thread::sleep(Duration::from_millis(10));
        }
    };
    assert!(r.ask(request(&font, Route::Font)));
    assert!(matches!(answer(&r), Loaded::Nothing));
    assert!(r.ask(request(&text, Route::Text)));
    assert_eq!(text_of(&answer(&r)), Some("after\n"));
}

#[test]
fn a_font_is_sampled_from_a_copy_the_reader_removes() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("face.ttf");
    std::fs::write(&path, "not really a font").expect("write");
    let seen = Arc::new(std::sync::Mutex::new(None));
    let saw = seen.clone();
    let parts = Parts {
        sample: Arc::new(move |copy, _, _| {
            let bytes = std::fs::read(copy).ok();
            *saw.lock().unwrap() = Some((copy.to_path_buf(), bytes));
            None
        }),
        cache: false,
        ..Parts::installed()
    };
    read(&parts, &mut Marks::default(), &request(&path, Route::Font));
    let (copy, bytes) = seen.lock().unwrap().take().expect("the sampler ran");
    assert_ne!(copy, path, "never the path on the mount");
    assert_eq!(bytes.as_deref(), Some(&b"not really a font"[..]));
    assert!(!copy.exists(), "the copy is gone");
}

#[test]
fn the_sweep_removes_copies_of_explorers_no_longer_running_and_long_extensions_are_dropped() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let dead = dir
        .path()
        .join(format!("{SCRATCH_PREFIX}{}-0.ttf", u32::MAX - 1));
    let mine = dir
        .path()
        .join(format!("{SCRATCH_PREFIX}{}-0.ttf", std::process::id()));
    let other = dir.path().join("keep.txt");
    for p in [&dead, &mine, &other] {
        std::fs::write(p, "x").expect("write");
    }
    sweep_scratch_in(dir.path());
    assert!(!dead.exists() && mine.exists() && other.exists());
    let copy = Scratch::of(Path::new("/x/a.averyveryverylongext"), b"x").expect("a copy");
    assert!(copy.0.extension().is_none(), "{:?}", copy.0);
}
