//! Where a face stands with the installed fonts, and Install: a copy into the user's fonts
//! folder (`$XDG_DATA_HOME/fonts`, which fontconfig reads) and `fc-cache` over it.

use reprisekit::controls::fontview::Install;
use reprisekit::text::fontfile::{self, FaceNames};
use std::path::{Path, PathBuf};

/// `$XDG_DATA_HOME/fonts`, else `~/.local/share/fonts`.
pub fn user_dir() -> Option<PathBuf> {
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))?;
    Some(data.join("fonts"))
}

/// Where `face` is installed, by its PostScript name (its family and style).
pub fn state(face: &FaceNames, user: Option<&Path>) -> Install {
    placed(&fontfile::installed(&face.postscript), user)
}

/// [`Install::User`] when one of `paths` is under the user's fonts folder `user` or the old
/// `~/.fonts`, [`Install::System`] when any is elsewhere, else [`Install::No`].
pub fn placed(paths: &[PathBuf], user: Option<&Path>) -> Install {
    let legacy = std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".fonts"));
    let mine = |p: &PathBuf| {
        user.is_some_and(|u| p.starts_with(u)) || legacy.as_ref().is_some_and(|l| p.starts_with(l))
    };
    if paths.iter().any(mine) {
        Install::User
    } else if paths.is_empty() {
        Install::No
    } else {
        Install::System
    }
}

/// The path `file` is copied to in `dir`: its own name, or "name (2).ext" and on while a
/// different file holds the name. A file with the same bytes is reused.
pub fn target(file: &Path, dir: &Path) -> std::io::Result<PathBuf> {
    let name = file
        .file_name()
        .map(PathBuf::from)
        .unwrap_or_else(|| "font".into());
    let stem = name
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = name
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    let bytes = std::fs::read(file)?;
    for n in 1.. {
        let at = match n {
            1 => dir.join(&name),
            n => dir.join(format!("{stem} ({n}){ext}")),
        };
        match std::fs::read(&at) {
            Ok(there) if there == bytes => return Ok(at),
            Ok(_) => continue,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(at),
            Err(_) => continue,
        }
    }
    unreachable!()
}

/// Copy `file` into `dir` (made if missing), then have fontconfig cache `dir`.
pub fn install(file: &Path, dir: &Path) -> Result<PathBuf, String> {
    let at = copy_in(file, dir).map_err(|e| e.to_string())?;
    fc_cache(dir);
    Ok(at)
}

/// `file` copied into `dir` at its [`target`], `dir` made if missing.
pub fn copy_in(file: &Path, dir: &Path) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let at = target(file, dir)?;
    if !at.exists() {
        std::fs::copy(file, &at)?;
    }
    Ok(at)
}

/// A failed `fc-cache` is logged only: fontconfig notices the folder changed on its next scan.
fn fc_cache(dir: &Path) {
    match std::process::Command::new("fc-cache").arg(dir).status() {
        Ok(s) if s.success() => {}
        Ok(s) => log::warn!("fc-cache {}: {s}", dir.display()),
        Err(e) => log::warn!("fc-cache {}: {e}", dir.display()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_face_is_the_users_system_or_not_installed_by_where_it_lies() {
        let user = Path::new("/home/u/.local/share/fonts");
        let sys = PathBuf::from("/usr/share/fonts/TTF/Sample.ttf");
        let mine = user.join("Sample.ttf");
        assert_eq!(placed(&[], Some(user)), Install::No);
        assert_eq!(
            placed(std::slice::from_ref(&sys), Some(user)),
            Install::System
        );
        assert_eq!(placed(&[sys, mine], Some(user)), Install::User);
    }

    #[test]
    fn install_copies_into_the_folder_and_numbers_a_clash() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("Sample.ttf");
        std::fs::write(&src, b"font one").unwrap();
        let dir = tmp.path().join("share/fonts");
        let at = copy_in(&src, &dir).unwrap();
        assert_eq!(at, dir.join("Sample.ttf"));
        assert_eq!(std::fs::read(&at).unwrap(), b"font one");
        assert_eq!(
            copy_in(&src, &dir).unwrap(),
            at,
            "the same bytes again reuse the file"
        );

        std::fs::write(&src, b"font two").unwrap();
        let second = copy_in(&src, &dir).unwrap();
        assert_eq!(second, dir.join("Sample (2).ttf"));
        assert_eq!(
            std::fs::read(&at).unwrap(),
            b"font one",
            "the first copy stays"
        );
    }

    #[test]
    fn the_user_fonts_folder_follows_xdg_data_home() {
        let want = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .map(|d| d.join("fonts"));
        let got = user_dir().unwrap();
        match want {
            Some(w) => assert_eq!(got, w),
            None => assert!(got.ends_with(".local/share/fonts"), "{got:?}"),
        }
    }
}
