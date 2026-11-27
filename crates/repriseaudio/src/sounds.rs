//! Event sounds: an XDG sound theme name or a file, played through `pw-play`.

use log::warn;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Sound {
    /// A sound theme name such as `message-new-instant`.
    Name(String),
    /// An absolute path to a sound file.
    File(PathBuf),
}

pub const FALLBACK_THEME: &str = "freedesktop";

const PLAYER: &str = "pw-play";

const EXTENSIONS: [&str; 3] = ["oga", "ogg", "wav"];

/// A file that has not finished playing by then is cut off.
const MAX_PLAY: Duration = Duration::from_secs(10);

/// When the sound playing now started; a slot older than [`MAX_PLAY`] plus [`STUCK_GRACE`] is
/// free, since its thread is stuck (a sound file on a dead mount blocks its `is_file`).
static PLAYING: Mutex<Option<Instant>> = Mutex::new(None);
const STUCK_GRACE: Duration = Duration::from_secs(5);

/// A name or theme that cannot leave the sounds directory.
fn safe_name(s: &str) -> bool {
    !s.is_empty() && s.len() <= 255 && !s.contains('/') && !s.starts_with('.')
}

/// `sounds` under `$XDG_DATA_HOME` and each of `$XDG_DATA_DIRS`, in lookup order.
pub fn sound_dirs() -> Vec<PathBuf> {
    (reprisexdg::data_home().into_iter())
        .chain(reprisexdg::data_dirs())
        .map(|d| d.join("sounds"))
        .collect()
}

/// The sound themes under `dirs`, sorted: each directory with an `index.theme` or a `stereo/`.
pub fn themes(dirs: &[PathBuf]) -> Vec<String> {
    let mut out: Vec<String> = dirs
        .iter()
        .filter_map(|d| std::fs::read_dir(d).ok())
        .flat_map(|rd| rd.flatten())
        .filter(|e| {
            let p = e.path();
            p.join("index.theme").is_file() || p.join("stereo").is_dir()
        })
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| safe_name(n))
        .collect();
    out.sort();
    out.dedup();
    out
}

/// The `Inherits` list of `theme`'s first `index.theme` in `dirs`.
fn inherits(dirs: &[PathBuf], theme: &str) -> Vec<String> {
    let Some(text) = dirs
        .iter()
        .find_map(|d| std::fs::read_to_string(d.join(theme).join("index.theme")).ok())
    else {
        return Vec::new();
    };
    let Some(line) = text
        .lines()
        .find_map(|l| l.trim().strip_prefix("Inherits="))
    else {
        return Vec::new();
    };
    line.split([',', ';'])
        .map(str::trim)
        .filter(|t| safe_name(t))
        .map(String::from)
        .collect()
}

/// The most themes one lookup walks, so a cycle or a deep chain ends.
const MAX_THEMES: usize = 10;

/// The file for sound `name` under `theme`: its stereo profile in each of `dirs`, then the themes
/// it inherits depth-first, then [`FALLBACK_THEME`]; a name with no file of its own tries its shorter dash
/// prefixes (`message-new-instant`, `message-new`, `message`).
pub fn resolve(dirs: &[PathBuf], theme: &str, name: &str) -> Option<PathBuf> {
    if !safe_name(name) {
        return None;
    }
    let mut themes: Vec<String> = Vec::new();
    let mut stack: Vec<String> = Vec::new();
    if safe_name(theme) {
        stack.push(theme.to_string());
    }
    while let Some(t) = stack.pop() {
        if themes.len() == MAX_THEMES {
            break;
        }
        if themes.contains(&t) {
            continue;
        }
        stack.extend(inherits(dirs, &t).into_iter().rev());
        themes.push(t);
    }
    if !themes.iter().any(|t| t == FALLBACK_THEME) {
        themes.push(FALLBACK_THEME.to_string());
    }
    let mut name = name;
    loop {
        for theme in &themes {
            for dir in dirs {
                let stereo = dir.join(theme).join("stereo");
                for ext in EXTENSIONS {
                    let path = stereo.join(format!("{name}.{ext}"));
                    if path.is_file() {
                        return Some(path);
                    }
                }
            }
        }
        name = &name[..name.rfind('-')?];
    }
}

/// A desktop moment with a sound of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Login,
    Logout,
    Error,
    Warning,
    DeviceAdded,
    DeviceRemoved,
    TrashEmpty,
    ScreenCapture,
    PowerPlug,
    PowerUnplug,
}

impl Event {
    /// Sound naming spec ids, tried in order: the later ones are what the freedesktop theme
    /// ships for a moment whose own id it lacks.
    pub fn names(self) -> &'static [&'static str] {
        match self {
            Event::Login => &["desktop-login", "service-login"],
            Event::Logout => &["desktop-logout", "service-logout"],
            Event::Error => &["dialog-error", "dialog-warning"],
            Event::Warning => &["dialog-warning"],
            Event::DeviceAdded => &["device-added"],
            Event::DeviceRemoved => &["device-removed"],
            Event::TrashEmpty => &["trash-empty"],
            Event::ScreenCapture => &["screen-capture", "camera-shutter"],
            Event::PowerPlug => &["power-plug"],
            Event::PowerUnplug => &["power-unplug"],
        }
    }

    /// The sound of a message box showing icon `icon`: only error and warning boxes have one.
    pub fn of_box_icon(icon: &str) -> Option<Event> {
        match icon {
            "dialog-error" => Some(Event::Error),
            "dialog-warning" => Some(Event::Warning),
            _ => None,
        }
    }
}

/// The number of [`Event`]s: the last one's index plus one.
const EVENTS: usize = Event::PowerUnplug as usize + 1;

/// An event comes again within this of its last sound silently, so a burst (a hub's devices, a
/// run of errors) sounds once.
pub const EVENT_GAP: Duration = Duration::from_secs(2);

/// When each [`Event`] last sounded.
#[derive(Debug, Default)]
pub struct Gaps([Option<Instant>; EVENTS]);

impl Gaps {
    /// Whether `event` at `now` sounds, recording it if so.
    pub fn due(&mut self, event: Event, now: Instant) -> bool {
        let last = &mut self.0[event as usize];
        if last.is_some_and(|t| now.saturating_duration_since(t) < EVENT_GAP) {
            return false;
        }
        *last = Some(now);
        true
    }
}

static GAPS: Mutex<Gaps> = Mutex::new(Gaps([None; EVENTS]));

/// Play `sound` from `theme` on a thread of its own; one plays at a time and a sound asked for while
/// another plays is skipped.
pub fn play(sound: Sound, theme: String) {
    spawn_player(move || match &sound {
        Sound::Name(name) => resolve(&sound_dirs(), &theme, name),
        Sound::File(path) => (path.is_absolute() && path.is_file()).then(|| path.clone()),
    });
}

/// [`play`] `event`'s sound from `theme`, unless it sounded within [`EVENT_GAP`].
pub fn play_event(event: Event, theme: String) {
    let due = (GAPS.lock().unwrap_or_else(|e| e.into_inner())).due(event, Instant::now());
    if due {
        spawn_player(move || {
            let dirs = sound_dirs();
            (event.names().iter()).find_map(|name| resolve(&dirs, &theme, name))
        });
    }
}

/// Whether a sound is playing, or about to.
pub fn playing() -> bool {
    PLAYING.lock().unwrap_or_else(|e| e.into_inner()).is_some()
}

/// Wait up to `max` for the sound playing now to end, so a session or program about to end is
/// heard to the end.
pub fn wait_quiet(max: Duration) {
    let until = Instant::now() + max;
    while playing() && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn spawn_player(find: impl FnOnce() -> Option<PathBuf> + Send + 'static) {
    let started = Instant::now();
    if !take_slot(
        &mut PLAYING.lock().unwrap_or_else(|e| e.into_inner()),
        started,
    ) {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("reprise-sound".into())
        .spawn(move || {
            if let Some(path) = find() {
                run(&path);
            }
            free_slot(started);
        });
    if spawned.is_err() {
        free_slot(started);
    }
}

/// Take the slot `busy` at `now` when no sound holds it or the one that does is stuck.
fn take_slot(busy: &mut Option<Instant>, now: Instant) -> bool {
    let free =
        busy.is_none_or(|since| now.saturating_duration_since(since) > MAX_PLAY + STUCK_GRACE);
    if free {
        *busy = Some(now);
    }
    free
}

/// Free the slot the sound started at `started` took, unless a later sound took it over.
fn free_slot(started: Instant) {
    let mut busy = PLAYING.lock().unwrap_or_else(|e| e.into_inner());
    if *busy == Some(started) {
        *busy = None;
    }
}

/// Set in every process cargo starts (`cargo test`, nextest, `cargo run`): such a process plays
/// nothing, so a test never sounds on the desktop running it.
const UNDER_CARGO: &str = "CARGO_MANIFEST_DIR";

/// Play `path` to its end or [`MAX_PLAY`]; whether the player started.
fn run(path: &Path) -> bool {
    if std::env::var_os(UNDER_CARGO).is_some() {
        log::debug!("not playing {} under cargo", path.display());
        return false;
    }
    let child = Command::new(PLAYER)
        .arg("--")
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    let mut child = match child {
        Ok(child) => child,
        Err(err) => {
            warn!("could not run {PLAYER}: {err}");
            return false;
        }
    };
    let until = Instant::now() + MAX_PLAY;
    while Instant::now() < until {
        match child.try_wait() {
            Ok(Some(_)) | Err(_) => return true,
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A temp tree removed on drop, so a failing test leaves nothing behind.
    struct Tree(PathBuf);

    impl std::ops::Deref for Tree {
        type Target = Path;
        fn deref(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn tree(files: &[&str]) -> Tree {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "repriseaudio-sounds-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        for f in files {
            let path = root.join(f);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, "").unwrap();
        }
        Tree(root)
    }

    #[test]
    fn only_error_and_warning_icons_have_a_box_sound() {
        assert_eq!(Event::of_box_icon("dialog-error"), Some(Event::Error));
        assert_eq!(Event::of_box_icon("dialog-warning"), Some(Event::Warning));
        assert_eq!(Event::of_box_icon("dialog-question"), None);
    }

    #[test]
    fn a_test_build_never_starts_the_player() {
        // The file is empty, so a player this does start is still silent.
        let root = tree(&["x.oga"]);
        let file = root.join("x.oga");
        assert!(!run(&file), "under cargo, {PLAYER} never starts");
    }

    #[test]
    fn a_sound_is_skipped_while_another_plays_until_that_one_is_stuck() {
        let t0 = Instant::now();
        let mut busy = None;
        assert!(take_slot(&mut busy, t0));
        assert!(!take_slot(&mut busy, t0 + MAX_PLAY), "still playing");
        let stuck = t0 + MAX_PLAY + STUCK_GRACE + Duration::from_millis(1);
        assert!(take_slot(&mut busy, stuck));
        assert_eq!(busy, Some(stuck));
    }

    #[test]
    fn an_event_within_the_gap_of_its_last_sound_is_silent() {
        let t0 = Instant::now();
        let mut gaps = Gaps::default();
        assert!(gaps.due(Event::DeviceAdded, t0));
        assert!(!gaps.due(Event::DeviceAdded, t0 + EVENT_GAP / 2));
        assert!(
            gaps.due(Event::DeviceRemoved, t0 + EVENT_GAP / 2),
            "another event has a gap of its own"
        );
        assert!(gaps.due(Event::DeviceAdded, t0 + EVENT_GAP));
        assert!(
            !gaps.due(Event::DeviceAdded, t0 + EVENT_GAP + EVENT_GAP / 2),
            "the gap runs from the last sound"
        );
    }

    #[test]
    fn an_event_takes_its_next_name_when_no_theme_has_the_first() {
        let root = tree(&[
            "sys/ocean/stereo/desktop-login.oga",
            "sys/freedesktop/stereo/service-login.oga",
            "sys/freedesktop/stereo/dialog-warning.oga",
        ]);
        let dirs = [root.join("sys")];
        let got = |theme: &str, event: Event| {
            (event.names().iter())
                .find_map(|n| resolve(&dirs, theme, n))
                .map(|p| p.strip_prefix(&*root).unwrap().to_path_buf())
        };
        assert_eq!(
            got("ocean", Event::Login),
            Some("sys/ocean/stereo/desktop-login.oga".into())
        );
        assert_eq!(
            got("freedesktop", Event::Login),
            Some("sys/freedesktop/stereo/service-login.oga".into())
        );
        assert_eq!(
            got("ocean", Event::Error),
            Some("sys/freedesktop/stereo/dialog-warning.oga".into())
        );
        assert_eq!(got("ocean", Event::TrashEmpty), None);
    }

    #[test]
    fn a_theme_name_resolves_through_inheritance_prefixes_and_the_fallback() {
        let root = tree(&[
            "user/ocean/stereo/bell.oga",
            "user/ocean/index.theme",
            "sys/deep/stereo/message-new.ogg",
            "sys/freedesktop/stereo/dialog-warning.oga",
            "sys/freedesktop/stereo/message-new-instant.oga",
        ]);
        std::fs::write(
            root.join("user/ocean/index.theme"),
            "[Sound Theme]\nName=Ocean\nInherits=deep\n",
        )
        .unwrap();
        let dirs = [root.join("user"), root.join("sys")];
        let got = |theme: &str, name: &str| {
            resolve(&dirs, theme, name).map(|p| p.strip_prefix(&*root).unwrap().to_path_buf())
        };
        assert_eq!(
            got("ocean", "bell"),
            Some("user/ocean/stereo/bell.oga".into())
        );
        assert_eq!(
            got("ocean", "message-new-instant"),
            Some("sys/freedesktop/stereo/message-new-instant.oga".into()),
            "the full name in the fallback comes before a shorter one"
        );
        assert_eq!(
            got("ocean", "message-new-email"),
            Some("sys/deep/stereo/message-new.ogg".into()),
            "a shorter name in the inherited theme"
        );
        assert_eq!(
            got("ocean", "dialog-warning"),
            Some("sys/freedesktop/stereo/dialog-warning.oga".into())
        );
        assert_eq!(
            got("missing", "message-new-instant"),
            Some("sys/freedesktop/stereo/message-new-instant.oga".into())
        );
        assert_eq!(got("ocean", "nothing-here"), None);
        assert_eq!(got("ocean", "../ocean/stereo/bell"), None);
        assert_eq!(
            got("../user/ocean", "bell"),
            None,
            "a theme that climbs out is the fallback alone"
        );
    }

    #[test]
    fn a_theme_follows_every_inherited_theme_down_the_chain() {
        let root = tree(&[
            "sys/ocean/index.theme",
            "sys/deep/index.theme",
            "sys/abyss/index.theme",
            "sys/abyss/stereo/bell.oga",
            "sys/coral/stereo/chime.oga",
            "sys/coral/stereo/bell.oga",
        ]);
        let index = |theme: &str, inherits: &str| {
            std::fs::write(
                root.join(format!("sys/{theme}/index.theme")),
                format!("[Sound Theme]\nInherits={inherits}\n"),
            )
            .unwrap();
        };
        index("ocean", "deep, coral");
        index("deep", "abyss");
        index("abyss", "ocean");
        let dirs = [root.join("sys")];
        let got = |name: &str| {
            resolve(&dirs, "ocean", name).map(|p| p.strip_prefix(&*root).unwrap().to_path_buf())
        };
        assert_eq!(
            got("chime"),
            Some("sys/coral/stereo/chime.oga".into()),
            "the second theme of the list"
        );
        assert_eq!(
            got("bell"),
            Some("sys/abyss/stereo/bell.oga".into()),
            "the first parent's own parents come before the second parent"
        );
        index("ocean", "deep");
        assert_eq!(
            got("bell"),
            Some("sys/abyss/stereo/bell.oga".into()),
            "the grandparent, through a cycle back to the start"
        );
    }

    #[test]
    fn a_theme_is_a_directory_with_an_index_or_a_stereo_profile() {
        let root = tree(&[
            "user/ocean/index.theme",
            "user/.cache/index.theme",
            "sys/deep/stereo/bell.oga",
            "sys/ocean/index.theme",
            "sys/notes/readme.txt",
            "sys/loose.oga",
        ]);
        let dirs = [root.join("user"), root.join("sys"), root.join("none")];
        assert_eq!(themes(&dirs), ["deep", "ocean"]);
    }
}
