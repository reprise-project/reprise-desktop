//! Event sounds: an XDG sound theme name or a file, played through `pw-play`.

use log::warn;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
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

static PLAYING: AtomicBool = AtomicBool::new(false);

/// A name or theme that cannot leave the sounds directory.
fn safe_name(s: &str) -> bool {
    !s.is_empty() && s.len() <= 255 && !s.contains('/') && !s.starts_with('.')
}

/// `sounds` under `$XDG_DATA_HOME` and each of `$XDG_DATA_DIRS`, in lookup order.
pub fn sound_dirs() -> Vec<PathBuf> {
    let var = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty());
    let home = var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| var("HOME").map(|h| PathBuf::from(h).join(".local/share")));
    let dirs = var("XDG_DATA_DIRS").unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    home.into_iter()
        .chain(std::env::split_paths(&dirs))
        .filter(|d| d.is_absolute())
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

/// Play `sound` from `theme` on a thread of its own; one plays at a time and a sound asked for while
/// another plays is skipped.
pub fn play(sound: Sound, theme: String) {
    if PLAYING.swap(true, Ordering::AcqRel) {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("reprise-sound".into())
        .spawn(move || {
            let path = match &sound {
                Sound::Name(name) => resolve(&sound_dirs(), &theme, name),
                Sound::File(path) => (path.is_absolute() && path.is_file()).then(|| path.clone()),
            };
            if let Some(path) = path {
                run(&path);
            }
            PLAYING.store(false, Ordering::Release);
        });
    if spawned.is_err() {
        PLAYING.store(false, Ordering::Release);
    }
}

fn run(path: &Path) {
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
            return;
        }
    };
    let until = Instant::now() + MAX_PLAY;
    while Instant::now() < until {
        match child.try_wait() {
            Ok(Some(_)) | Err(_) => return,
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(files: &[&str]) -> PathBuf {
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
        root
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
            resolve(&dirs, theme, name).map(|p| p.strip_prefix(&root).unwrap().to_path_buf())
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
        std::fs::remove_dir_all(&root).unwrap();
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
            resolve(&dirs, "ocean", name).map(|p| p.strip_prefix(&root).unwrap().to_path_buf())
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
        std::fs::remove_dir_all(&root).unwrap();
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
        std::fs::remove_dir_all(&root).unwrap();
    }
}
