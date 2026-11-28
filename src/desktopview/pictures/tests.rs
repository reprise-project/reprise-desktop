use super::*;

const GREEN: u32 = 0xff20_a040;

fn thumb(width: u32, height: u32, dims: Option<(u32, u32)>) -> thumbpipe::Thumb {
    thumbpipe::Thumb {
        width,
        height,
        pixels: vec![GREEN; (width * height) as usize],
        dims,
    }
}

/// The rows and columns of `c` holding any ink, as (x, y, w, h).
fn inked(c: &Canvas) -> (i32, i32, i32, i32) {
    let (w, h) = (c.width as i32, c.height as i32);
    let ink = |x: i32, y: i32| c.get(x, y) >> 24 != 0;
    let cols: Vec<i32> = (0..w).filter(|&x| (0..h).any(|y| ink(x, y))).collect();
    let rows: Vec<i32> = (0..h).filter(|&y| (0..w).any(|x| ink(x, y))).collect();
    let (x0, y0) = (cols[0], rows[0]);
    (
        x0,
        y0,
        cols[cols.len() - 1] - x0 + 1,
        rows[rows.len() - 1] - y0 + 1,
    )
}

#[test]
fn a_picture_fits_the_icon_square_centred_and_never_grows() {
    let wide = icon_of(&thumb(128, 64, Some((800, 400))), 1);
    assert_eq!((wide.width, wide.height), (32, 32));
    assert_eq!(inked(&wide), (0, 8, 32, 16));
    assert_eq!(wide.get(16, 16), GREEN);

    let small = icon_of(&thumb(10, 5, Some((10, 5))), 1);
    assert_eq!(inked(&small), (11, 13, 10, 5), "at its own size");

    let tall = icon_of(&thumb(64, 128, None), 2);
    assert_eq!((tall.width, tall.height), (64, 64), "device pixels");
    assert_eq!(inked(&tall), (16, 0, 32, 64));
}

/// A folder's cell wearing its picture, at scale 1 and 2.
#[test]
fn a_desktop_cell_wears_its_folders_picture() {
    reprisekit::pins::fonts();
    let mut got = Vec::new();
    for scale in [1, 2] {
        let theme = reprisekit::theme::Theme::default().scaled(scale);
        let text = reprisekit::text::TextRenderer::load(&theme.fonts);
        let icon = icon_of(&thumb(128, 96, Some((640, 480))), scale);
        let cell = |icon: Option<&Canvas>| {
            let text = text.for_output(theme.scale);
            crate::desktop::draw_cell(&theme, text, "Trip", icon, false, None, None, None)
        };
        let (worn, plain) = (cell(Some(&icon)), cell(None));
        assert_ne!(worn.pixels, plain.pixels, "the picture shows");
        let hash = worn.pixels.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, p| {
            (h ^ u64::from(*p)).wrapping_mul(0x0100_0000_01b3)
        });
        got.push((format!("scale {scale}"), hash));
    }
    let pins = reprisekit::crate_dir!().join("tests/pins/desktop_picture.txt");
    reprisekit::pins::check_file(&pins, &got);
}

#[test]
fn a_picture_is_asked_for_again_at_a_new_size_or_after_it_gave_none() {
    let (a, b) = (PathBuf::from("/p/a.png"), PathBuf::from("/p/b.png"));
    let mut p = Pictures::default();
    let t0 = Instant::now();
    assert_eq!(p.wanted([&a, &b, &a], 32, t0), [a.clone(), b.clone()]);
    p.land(a.clone(), 32, Some(Canvas::new(32, 32)), t0);
    p.land(b.clone(), 32, None, t0);
    assert!(p.wanted([&a, &b], 32, t0).is_empty());
    assert!(p.shown(&a, 32).is_some() && p.shown(&b, 32).is_none());
    assert!(p.shown(&a, 64).is_none(), "another size");
    assert_eq!(p.wanted([&a, &b], 32, t0 + RETRY_AFTER), vec![b.clone()]);
    assert_eq!(p.wanted([&a, &b], 64, t0), [a.clone(), b.clone()]);

    assert!(p.wanted([&b], 32, t0).is_empty());
    assert!(
        p.shown(&a, 32).is_none(),
        "forgotten once no folder names it"
    );
}

/// A helper that answers the first four requests with a 2x1 thumbnail of a 4x2 picture, from a
/// shell, and ends once its stdin closes.
fn answering() -> Command {
    let one = r"\002\0\0\0\001\0\0\0\004\0\0\0\002\0\0\0\100\240\040\377\100\240\040\377";
    let mut c = Command::new("sh");
    c.args([
        "-c",
        &format!("for i in 1 2 3 4; do printf '{one}'; done; exec cat >/dev/null"),
    ]);
    c
}

fn fetched(
    command: impl Fn() -> Command,
    paths: &[PathBuf],
    within: Duration,
) -> Vec<(PathBuf, Option<(u32, u32)>)> {
    let mut got = Vec::new();
    let stop = AtomicBool::new(false);
    fetch(command, paths, 128, &stop, within, |p, t| {
        got.push((p, t.map(|t| (t.width, t.height))));
    });
    got
}

#[test]
fn every_picture_gets_one_answer_in_order() {
    use std::os::unix::ffi::OsStrExt;
    let nul = PathBuf::from(std::ffi::OsStr::from_bytes(b"/p/a\0.png"));
    let paths = [PathBuf::from("/p/a.png"), nul, PathBuf::from("/p/b.png")];
    let got = fetched(answering, &paths, Duration::from_secs(5));
    assert_eq!(
        got,
        [
            (paths[0].clone(), Some((2, 1))),
            (paths[1].clone(), None),
            (paths[2].clone(), Some((2, 1))),
        ],
        "a path the pipe cannot carry gets none and the rest still answer"
    );
}

#[test]
fn a_helper_that_never_answers_is_given_up_on_and_the_rest_go_on() {
    let starts = std::cell::Cell::new(0);
    let hangs_first = || {
        starts.set(starts.get() + 1);
        match starts.get() {
            1 => {
                let mut c = Command::new("sleep");
                c.arg("60");
                c
            }
            _ => answering(),
        }
    };
    let paths = [PathBuf::from("/dead/a.png"), PathBuf::from("/p/b.png")];
    let asked = Instant::now();
    let got = fetched(hangs_first, &paths, Duration::from_millis(200));
    assert!(
        asked.elapsed() < Duration::from_secs(5),
        "{:?}",
        asked.elapsed()
    );
    assert_eq!(
        got,
        [(paths[0].clone(), None), (paths[1].clone(), Some((2, 1)))]
    );
    assert_eq!(starts.get(), 2, "a fresh helper for the rest");
}

#[test]
fn past_three_failures_the_rest_are_left_for_the_next_round() {
    let starts = std::cell::Cell::new(0);
    let never = || {
        starts.set(starts.get() + 1);
        let mut c = Command::new("sleep");
        c.arg("60");
        c
    };
    let paths: Vec<PathBuf> = (0..5)
        .map(|i| PathBuf::from(format!("/dead/{i}.png")))
        .collect();
    let got = fetched(never, &paths, Duration::from_millis(50));
    assert!(got.iter().all(|(_, t)| t.is_none()));
    let asked: Vec<&PathBuf> = got.iter().map(|(p, _)| p).collect();
    assert_eq!(
        asked,
        paths.iter().take(GIVE_UP as usize).collect::<Vec<_>>()
    );
    assert_eq!(starts.get(), GIVE_UP);
}

#[test]
fn a_helper_that_cannot_start_gives_none_for_the_whole_round() {
    let starts = std::cell::Cell::new(0);
    let missing = || {
        starts.set(starts.get() + 1);
        Command::new("/nonexistent/reprise-explorer")
    };
    let paths: Vec<PathBuf> = (0..5)
        .map(|i| PathBuf::from(format!("/p/{i}.png")))
        .collect();
    let got = fetched(missing, &paths, Duration::from_secs(5));
    let gave_none: Vec<PathBuf> = got
        .into_iter()
        .map(|(p, t)| (p, t.is_none()))
        .filter(|x| x.1)
        .map(|x| x.0)
        .collect();
    assert_eq!(gave_none, paths, "every path gives none");
    assert_eq!(starts.get(), 1, "one try");
}

#[test]
fn a_stopped_round_answers_nothing_it_did_not_ask() {
    let mut got = Vec::new();
    let stop = AtomicBool::new(true);
    let paths = [PathBuf::from("/p/a.png")];
    fetch(
        answering,
        &paths,
        32,
        &stop,
        Duration::from_secs(5),
        |p, _| got.push(p),
    );
    assert!(got.is_empty());
}

/// The helper's slowest answer: its open and read of the file, then a thumbnailer slot wait, a
/// fresh helper's jail probe and the jailed run, as `repriseapp::thumbs` and
/// `reprisefs::thumbnailers` bound them.
#[test]
fn the_answer_limit_covers_the_helpers_slowest_answer() {
    let reads = 2 * Duration::from_secs(3);
    let jail = 3 * reprisefs::thumbnailers::DEADLINE;
    assert!(ANSWER_WITHIN > reads + jail, "{ANSWER_WITHIN:?}");
}

#[test]
fn pictures_that_gave_none_are_asked_after_the_untried() {
    let (a, c) = (PathBuf::from("/p/a.png"), PathBuf::from("/p/c.png"));
    let mut p = Pictures::default();
    let t0 = Instant::now();
    p.land(a.clone(), 32, None, t0);
    assert_eq!(p.wanted([&a, &c], 32, t0), vec![c.clone()]);
    assert_eq!(p.wanted([&a, &c], 32, t0 + RETRY_AFTER), [c, a]);
}

/// A folder's cell draws its icon until its picture lands at the desktop's size, then wears it.
#[test]
fn a_folder_icon_wears_its_picture_once_it_lands() {
    let mut h = crate::harness::Harness::new();
    h.output("TEST-1", 1024, 768);
    let dir = std::env::temp_dir().join(format!("reprise-desk-picture-{}", std::process::id()));
    crate::desktopview::create(&mut h.state, dir.clone(), Default::default(), None);
    let picture = PathBuf::from("/p/trip.png");
    let view = h.state.desktop.as_mut().unwrap();
    view.entries.push(crate::desktop::DesktopEntry {
        name: "Trip".into(),
        label: "Trip".into(),
        icon: "folder".into(),
        target: crate::desktop::Target::Open(dir.join("Trip")),
        path: dir.join("Trip"),
        shortcut: false,
        kind: crate::desktop::Kind::File,
        is_dir: true,
        allow_launching: false,
        picture: Some(picture.clone()),
    });
    // A round already running holds off another, so nothing but the landing below lands.
    view.pictures.round = Some((ICON_PX, Arc::new(AtomicBool::new(false))));
    h.state.relayout_desktop();
    let key = |h: &crate::harness::Harness| {
        let view = h.state.desktop.as_ref().unwrap();
        view.drawn[0].as_ref().map(|k| k.picture)
    };
    assert_eq!(key(&h), Some(None), "the icon meanwhile");

    h.state
        .desktop_picture_landed(picture.clone(), ICON_PX, None);
    assert_eq!(key(&h), Some(None), "none came");
    h.state
        .desktop_picture_landed(picture.clone(), ICON_PX, Some(Canvas::new(32, 32)));
    let serial = key(&h).flatten().expect("the picture is worn");
    h.state
        .desktop_picture_landed(picture, ICON_PX, Some(Canvas::new(32, 32)));
    assert_ne!(key(&h).flatten(), Some(serial), "a new landing redraws");
}
