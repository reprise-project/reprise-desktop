//! What a package file holds, read with the system's own tools in the C locale (`pacman -Qip`,
//! `dpkg-deb --field`), and an Arch package installed through `pkexec pacman -U`.

use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `.flatpakref`: an app in a repository.
    FlatpakRef,
    /// `.flatpak`: a single-file bundle.
    FlatpakBundle,
    /// `.pkg.tar.zst` (or `.xz`), for pacman.
    Pacman,
    Deb,
}

/// What `path`'s name says it is.
pub fn kind_of(path: &Path) -> Option<Kind> {
    let name = path.file_name()?.to_str()?.to_ascii_lowercase();
    Some(if name.ends_with(".flatpakref") {
        Kind::FlatpakRef
    } else if name.ends_with(".flatpak") {
        Kind::FlatpakBundle
    } else if name.ends_with(".pkg.tar.zst") || name.ends_with(".pkg.tar.xz") {
        Kind::Pacman
    } else if name.ends_with(".deb") {
        Kind::Deb
    } else {
        return None;
    })
}

/// A package file copied into a fresh folder only this user can open, which is deleted when the
/// copy drops. The sheet reads and installs the copy, so a file swapped after it opened changes
/// nothing.
#[derive(Debug)]
pub struct Copy {
    dir: PathBuf,
    /// The copy, under the file's own name.
    pub file: PathBuf,
    /// A detached `NAME.sig` lay beside the file and was copied with it.
    pub signed: bool,
}

impl Drop for Copy {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Copy the regular file `path`, and its `.sig` if there is one, into a new 0700 folder in
/// `parent`.
pub fn private_copy(path: &Path, parent: &Path) -> io::Result<Copy> {
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no file name"))?;
    let dir = fresh_dir(parent)?;
    let mut copy = Copy {
        file: dir.join(name),
        dir,
        signed: false,
    };
    copy_regular(path, &copy.file)?;
    let sig = |p: &Path| {
        let mut s = p.as_os_str().to_owned();
        s.push(".sig");
        PathBuf::from(s)
    };
    copy.signed = copy_regular(&sig(path), &sig(&copy.file)).is_ok();
    Ok(copy)
}

fn fresh_dir(parent: &Path) -> io::Result<PathBuf> {
    use std::hash::{BuildHasher, Hasher};
    use std::os::unix::fs::DirBuilderExt;
    let mut tries = 0;
    loop {
        let tag = std::hash::RandomState::new().build_hasher().finish();
        let dir = parent.join(format!("reprise-install-{tag:016x}"));
        match std::fs::DirBuilder::new().mode(0o700).create(&dir) {
            Ok(()) => return Ok(dir),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists && tries < 16 => tries += 1,
            Err(e) => return Err(e),
        }
    }
}

/// Copy `from` into the new file `to`; a FIFO or device is refused before it is read.
fn copy_regular(from: &Path, to: &Path) -> io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut src = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOCTTY)
        .open(from)?;
    if !src.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a regular file",
        ));
    }
    let mut dst = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(to)?;
    io::copy(&mut src, &mut dst)?;
    Ok(())
}

/// A package file's name, version, installed size in bytes and who built it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Facts {
    pub name: String,
    pub version: String,
    pub size: Option<u64>,
    pub packager: String,
}

/// A size as `pacman` or GLib writes it ("812.18 KiB", "34.2 MB" with a no-break space), in bytes.
pub fn size_bytes(text: &str) -> Option<u64> {
    let (n, unit) = text.trim().split_once(char::is_whitespace)?;
    let n: f64 = n.parse().ok()?;
    let mul = match unit.trim() {
        "B" | "bytes" | "byte" => 1.0,
        "KiB" => 1024.0,
        "MiB" => 1024.0 * 1024.0,
        "GiB" => 1024.0 * 1024.0 * 1024.0,
        "kB" => 1e3,
        "MB" => 1e6,
        "GB" => 1e9,
        _ => return None,
    };
    Some((n * mul) as u64)
}

/// `pacman -Qip`'s `Key  : value` lines.
fn parse_pacman(out: &str) -> Facts {
    let mut f = Facts::default();
    for line in out.lines() {
        let Some((k, v)) = line.split_once(" : ") else {
            continue;
        };
        let v = v.trim();
        match k.trim() {
            "Name" => f.name = v.into(),
            "Version" => f.version = v.into(),
            "Installed Size" => f.size = size_bytes(v),
            "Packager" => f.packager = v.into(),
            _ => {}
        }
    }
    f
}

/// `dpkg-deb --field`'s `Key: value` lines; `Installed-Size` is in KiB.
fn parse_deb(out: &str) -> Facts {
    let mut f = Facts::default();
    for line in out.lines() {
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        let v = v.trim();
        match k {
            "Package" => f.name = v.into(),
            "Version" => f.version = v.into(),
            "Installed-Size" => f.size = v.parse::<u64>().ok().map(|k| k * 1024),
            "Maintainer" => f.packager = v.into(),
            _ => {}
        }
    }
    f
}

fn output(program: &str, args: &[&OsStr]) -> Result<String, String> {
    let out = Command::new(program)
        .args(args)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .output()
        .map_err(|_| format!("{program} is not installed, so Reprise cannot read this package."))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        let line = err.lines().map(str::trim).rfind(|l| !l.is_empty());
        Err(line.unwrap_or("The package could not be read.").to_string())
    }
}

/// Read package file `path` of `kind` (`Pacman` or `Deb`).
pub fn read(path: &Path, kind: Kind) -> Result<Facts, String> {
    let p = path.as_os_str();
    let os = OsStr::new;
    let facts = match kind {
        Kind::Pacman => parse_pacman(&output("pacman", &[os("-Qip"), os("--"), p])?),
        Kind::Deb => {
            let fields = ["Package", "Version", "Installed-Size", "Maintainer"];
            let mut args = vec![os("--field"), p];
            args.extend(fields.map(os));
            parse_deb(&output("dpkg-deb", &args)?)
        }
        Kind::FlatpakRef | Kind::FlatpakBundle => return Err("Not a package file.".into()),
    };
    if facts.name.is_empty() {
        return Err("The package does not name itself.".into());
    }
    Ok(facts)
}

/// The packages `pacman -U path` would install, by name: the file's and those it pulls in.
pub fn pacman_targets(path: &Path) -> Result<Vec<String>, String> {
    let os = OsStr::new;
    let out = output(
        "pacman",
        &[
            os("-Up"),
            os("--print-format"),
            os("%n"),
            os("--"),
            path.as_os_str(),
        ],
    )?;
    Ok(out
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect())
}

/// Why `pkexec pacman -U` did not install.
#[derive(Debug, PartialEq, Eq)]
pub enum PacmanError {
    /// pkexec's password prompt was dismissed.
    Dismissed,
    Failed(String),
}

/// pkexec's exit status when its prompt was dismissed.
const PKEXEC_DISMISSED: i32 = 126;

/// A step pacman reports, "installing htop..." as "Installing htop".
fn pacman_step(line: &str) -> Option<String> {
    let line = line.trim();
    if line.starts_with("::") {
        return None;
    }
    let step = line.strip_suffix("...")?;
    let mut s = step.to_string();
    if let Some(first) = s.get(..1) {
        s.replace_range(..1, &first.to_uppercase());
    }
    Some(s)
}

/// Install `path` with `pkexec pacman -U`, as root once polkit allows it, calling `step` with each
/// step pacman reports.
pub fn pacman_install(path: &Path, step: &mut dyn FnMut(String)) -> Result<(), PacmanError> {
    use std::io::{BufRead, BufReader, Read};
    let mut child = Command::new("pkexec")
        .args(["pacman", "-U", "--noconfirm", "--"])
        .arg(path)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| PacmanError::Failed(format!("pkexec could not be started: {e}.")))?;
    // stderr drains on its own thread, so a chatty one never fills its pipe.
    let err = child.stderr.take().map(|mut e| {
        std::thread::spawn(move || {
            let mut s = String::new();
            let _ = e.read_to_string(&mut s);
            s
        })
    });
    if let Some(out) = child.stdout.take() {
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            if let Some(s) = pacman_step(&line) {
                step(s);
            }
        }
    }
    let status = child
        .wait()
        .map_err(|e| PacmanError::Failed(e.to_string()))?;
    let stderr = err.and_then(|t| t.join().ok()).unwrap_or_default();
    match status.code() {
        Some(0) => Ok(()),
        Some(PKEXEC_DISMISSED) => Err(PacmanError::Dismissed),
        _ => {
            let line = stderr.lines().map(str::trim).rfind(|l| !l.is_empty());
            let line = line.map(|l| l.strip_prefix("error: ").unwrap_or(l));
            Err(PacmanError::Failed(
                line.unwrap_or("pacman did not install the package.")
                    .to_string(),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_name_says_what_kind_of_file_it_is() {
        let k = |n: &str| kind_of(Path::new(n));
        assert_eq!(k("/d/foot-1.28.0-2-x86_64.pkg.tar.zst"), Some(Kind::Pacman));
        assert_eq!(k("/d/old-1-1-any.pkg.tar.xz"), Some(Kind::Pacman));
        assert_eq!(k("/d/hello_2.10-3_amd64.DEB"), Some(Kind::Deb));
        assert_eq!(k("/d/org.example.App.flatpakref"), Some(Kind::FlatpakRef));
        assert_eq!(k("/d/org.example.App.flatpak"), Some(Kind::FlatpakBundle));
        assert_eq!(k("/d/backup.tar.zst"), None);
    }

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("reprise-pkgfile-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn the_private_copy_keeps_what_the_file_held_when_it_opened() {
        use std::os::unix::fs::PermissionsExt;
        let from = scratch("copy-from");
        let to = scratch("copy-to");
        let pkg = from.join("foot-1.28.0-2-x86_64.pkg.tar.zst");
        std::fs::write(&pkg, "first").unwrap();
        let copy = private_copy(&pkg, &to).unwrap();
        std::fs::write(&pkg, "swapped").unwrap();
        assert_eq!(copy.file.file_name(), pkg.file_name());
        assert_eq!(std::fs::read_to_string(&copy.file).unwrap(), "first");
        assert!(!copy.signed);
        let dir = copy.file.parent().unwrap().to_path_buf();
        assert_eq!(dir.parent(), Some(to.as_path()));
        let mode = std::fs::metadata(&dir).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700, "{mode:o}");
        drop(copy);
        assert!(!dir.exists(), "the copy's folder goes with it");

        std::fs::write(from.join("foot-1.28.0-2-x86_64.pkg.tar.zst.sig"), "sig").unwrap();
        let signed = private_copy(&pkg, &to).unwrap();
        assert!(signed.signed);
        let sig = signed
            .file
            .with_file_name("foot-1.28.0-2-x86_64.pkg.tar.zst.sig");
        assert_eq!(std::fs::read_to_string(sig).unwrap(), "sig");
        drop(signed);

        let fifo = from.join("pipe.deb");
        let c = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
        // SAFETY: mkfifo reads the NUL-terminated path only during the call.
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
        let refused = private_copy(&fifo, &to).unwrap_err();
        assert_eq!(refused.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(
            std::fs::read_dir(&to).unwrap().count(),
            0,
            "no folder left behind"
        );
        let _ = std::fs::remove_dir_all(&from);
        let _ = std::fs::remove_dir_all(&to);
    }

    #[test]
    fn pacman_steps_read_as_sentences() {
        assert_eq!(
            pacman_step("installing htop...").as_deref(),
            Some("Installing htop")
        );
        assert_eq!(
            pacman_step("checking package integrity...").as_deref(),
            Some("Checking package integrity")
        );
        assert_eq!(pacman_step(":: Processing package changes..."), None);
        assert_eq!(pacman_step("Packages (1) htop-3.5.3-1"), None);
    }

    #[test]
    fn sizes_read_in_either_units() {
        assert_eq!(size_bytes("812.18 KiB"), Some(831_672));
        assert_eq!(size_bytes("34.2\u{a0}MB"), Some(34_200_000));
        assert_eq!(size_bytes("2.00 MiB"), Some(2 * 1024 * 1024));
        assert_eq!(size_bytes("lots"), None);
    }

    #[test]
    fn pacman_and_dpkg_name_the_package() {
        let pacman = "Name            : foot\n\
                      Version         : 1.28.0-2\n\
                      Description     : Fast, lightweight, and minimalistic Wayland terminal\n\
                      Installed Size  : 812.18 KiB\n\
                      Packager        : CachyOS <admin@cachyos.org>\n\
                      Build Date      : Tue 01 Sep 2026 10:00:00 AM UTC\n";
        assert_eq!(
            parse_pacman(pacman),
            Facts {
                name: "foot".into(),
                version: "1.28.0-2".into(),
                size: Some(831_672),
                packager: "CachyOS <admin@cachyos.org>".into(),
            }
        );
        let deb = "Package: hello\nVersion: 2.10-3\nInstalled-Size: 280\n\
                   Maintainer: Santiago Vila <sanvila@debian.org>\n";
        assert_eq!(
            parse_deb(deb),
            Facts {
                name: "hello".into(),
                version: "2.10-3".into(),
                size: Some(280 * 1024),
                packager: "Santiago Vila <sanvila@debian.org>".into(),
            }
        );
    }
}
