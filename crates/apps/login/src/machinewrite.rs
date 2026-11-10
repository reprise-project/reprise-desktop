//! Writes the machine settings for `reprise-login-settings`, and the login screen's picture.

use std::fs::{File, OpenOptions, Permissions};
use std::io;
use std::os::fd::AsRawFd as _;
use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use image::imageops::{self, FilterType};
use image::{GenericImage as _, GenericImageView as _, ImageBuffer, Pixel};
use repriselogin::machine::{
    Background, MAX_PICTURE, MachineConfig, PICTURE_MAX_SIDE, decode_picture, file_tag,
    parse_save_request,
};

use crate::atomic;

/// The longest settings text the helper reads, in bytes.
pub const MAX_INPUT: usize = 16 * 1024;
/// The longest save request: the settings, a picture and the lines before them.
pub const MAX_REQUEST: usize = MAX_INPUT + MAX_PICTURE + 256;
/// The most pixels a stored picture holds, a 7680x4320 screen's worth in any shape.
pub const STORED_PIXELS: u64 = 7680 * 4320;
/// The largest stored picture in bytes: a PNG of [`STORED_PIXELS`] 8-bit RGBA pixels in stored
/// deflate blocks, the most the encoder writes, is under 127 MiB.
pub const MAX_STORED: u64 = 160 << 20;

/// How long a save waits for another one to finish before it gives up.
pub const LOCK_WAIT: Duration = Duration::from_secs(5);
const LOCK_POLL: Duration = Duration::from_millis(50);

/// Why [`apply`] failed.
#[derive(Debug)]
pub enum Error {
    /// Every warning the input raised.
    Invalid(Vec<String>),
    /// The file is no longer the one the settings were read from.
    Changed,
    /// Another save held the lock for all of [`LOCK_WAIT`].
    Busy,
    Io(io::Error),
}

fn invalid(why: impl Into<String>) -> Error {
    Error::Invalid(vec![why.into()])
}

/// Atomically replaces `path` with `input`'s settings while it has the tag they expect, then
/// `picture` with their picture as a PNG; settings showing none remove it. One save at a time.
pub fn apply(input: &[u8], path: &Path, picture: &Path) -> Result<(), Error> {
    apply_waiting(input, path, picture, LOCK_WAIT)
}

fn apply_waiting(input: &[u8], path: &Path, picture: &Path, wait: Duration) -> Result<(), Error> {
    let req = parse_save_request(input).map_err(invalid)?;
    if req.toml.len() > MAX_INPUT {
        return Err(invalid("the settings are too long"));
    }
    let (config, warnings) = MachineConfig::parse(req.toml);
    if !warnings.is_empty() {
        return Err(Error::Invalid(warnings));
    }
    let shown = config.login.background == Some(Background::Picture);
    let png = match req.picture {
        Some(_) if !shown => {
            return Err(invalid("a picture came with settings that do not show it"));
        }
        Some(bytes) => Some(stored(bytes).map_err(invalid)?),
        None => None,
    };
    let _lock = lock(path, wait)?;
    let now = match std::fs::read(path) {
        Ok(bytes) => Some(bytes),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(Error::Io(e)),
    };
    if file_tag(now.as_deref()) != req.tag {
        return Err(Error::Changed);
    }
    let staged = match png {
        Some(png) => {
            let dir = picture.parent().unwrap_or(Path::new("."));
            atomic::make_dir(dir, 0o711).map_err(Error::Io)?;
            traversable(dir).map_err(Error::Io)?;
            Some(atomic::stage(picture, &png, 0o711, 0o644).map_err(Error::Io)?)
        }
        None if shown && !std::fs::symlink_metadata(picture).is_ok_and(|m| m.is_file()) => {
            return Err(invalid("there is no picture for the background; pick one"));
        }
        None => None,
    };
    atomic::replace(path, config.to_toml().as_bytes(), 0o755, 0o644).map_err(Error::Io)?;
    if let Some(staged) = staged {
        staged.commit().map_err(Error::Io)?;
    }
    if !shown {
        match std::fs::remove_file(picture) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(Error::Io(e)),
            _ => {}
        }
    }
    Ok(())
}

/// `bytes` as the PNG the login screen reads: scaled down to [`stored_size`], upright, 8 bits a
/// channel, and none of the original's metadata, since the copy is world-readable.
fn stored(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let (picture, orientation) = decode_picture(bytes)?;
    let (w, h) = stored_size(picture.width(), picture.height());
    let mut picture = match picture.color().has_alpha() {
        true => image::DynamicImage::ImageRgba8(shrink(picture.into_rgba8(), w, h)),
        false => image::DynamicImage::ImageRgb8(shrink(picture.into_rgb8(), w, h)),
    };
    picture.apply_orientation(orientation);
    let mut out = io::Cursor::new(Vec::new());
    (picture.write_to(&mut out, image::ImageFormat::Png))
        .map_err(|e| format!("the picture could not be stored: {e}"))?;
    let png = out.into_inner();
    if png.len() as u64 > MAX_STORED {
        return Err(format!(
            "the picture is over {} MB once stored",
            MAX_STORED >> 20
        ));
    }
    Ok(png)
}

/// `w` x `h` scaled down, keeping its shape, to at most [`STORED_PIXELS`] pixels and
/// [`PICTURE_MAX_SIDE`] a side; a size within both is kept.
pub fn stored_size(w: u32, h: u32) -> (u32, u32) {
    let pixels = u64::from(w) * u64::from(h);
    let by_pixels = (STORED_PIXELS as f64 / pixels as f64).sqrt();
    let by_side = f64::from(PICTURE_MAX_SIDE) / f64::from(w.max(h));
    let s = by_pixels.min(by_side);
    match s < 1.0 {
        true => ((f64::from(w) * s) as u32, (f64::from(h) * s) as u32),
        false => (w, h),
    }
}

/// `src` resampled to `w` x `h` with a triangle filter, a band at a time: `imageops::resize`
/// keeps an `f32` RGBA copy of its first pass, 16 bytes a pixel.
fn shrink<P: Pixel<Subpixel = u8> + 'static>(
    src: ImageBuffer<P, Vec<u8>>,
    w: u32,
    h: u32,
) -> ImageBuffer<P, Vec<u8>> {
    const BAND: u32 = 32;
    let (sw, sh) = src.dimensions();
    if (w, h) == (sw, sh) {
        return src;
    }
    let mut tall = ImageBuffer::new(sw, h);
    for x in (0..sw).step_by(BAND as usize) {
        let bw = BAND.min(sw - x);
        let band = imageops::resize(&*src.view(x, 0, bw, sh), bw, h, FilterType::Triangle);
        tall.copy_from(&band, x, 0).expect("the band lies inside");
    }
    drop(src);
    let mut out = ImageBuffer::new(w, h);
    for y in (0..h).step_by(BAND as usize) {
        let bh = BAND.min(h - y);
        let band = imageops::resize(&*tall.view(0, y, sw, bh), w, bh, FilterType::Triangle);
        out.copy_from(&band, 0, y).expect("the band lies inside");
    }
    out
}

/// Lets every account pass through `dir`, which the greeter's account reads the picture in.
fn traversable(dir: &Path) -> io::Result<()> {
    let mode = std::fs::symlink_metadata(dir)?.permissions().mode() & 0o7777;
    if mode & 0o011 != 0o011 {
        std::fs::set_permissions(dir, Permissions::from_mode(mode | 0o011))?;
    }
    Ok(())
}

/// `.NAME.lock` beside `path`.
pub fn lock_path(path: &Path) -> PathBuf {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    path.with_file_name(format!(".{name}.lock"))
}

/// An exclusive `flock` on [`lock_path`], made 0600 when missing, held until the file drops.
/// The directory itself is not locked: any account can open it, and so hold its lock.
fn lock(path: &Path, wait: Duration) -> Result<File, Error> {
    let dir = path.parent().unwrap_or(Path::new("."));
    atomic::make_dir(dir, 0o755).map_err(Error::Io)?;
    let f = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(lock_path(path))
        .map_err(Error::Io)?;
    f.set_permissions(Permissions::from_mode(0o600))
        .map_err(Error::Io)?;
    let until = Instant::now() + wait;
    loop {
        // SAFETY: flock on a descriptor this function owns.
        if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            return Ok(f);
        }
        let e = io::Error::last_os_error();
        match e.raw_os_error() {
            Some(libc::EWOULDBLOCK) if Instant::now() >= until => return Err(Error::Busy),
            Some(libc::EWOULDBLOCK) => std::thread::sleep(LOCK_POLL),
            Some(libc::EINTR) => {}
            _ => return Err(Error::Io(e)),
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use repriselogin::machine::MachineRemote;

    use super::*;
    use repriselogin::machine::{BACKGROUND_PICTURE, save_request};

    /// `toml` as a save request expecting `path` as it is now.
    fn req(path: &Path, toml: &str) -> Vec<u8> {
        save_request(&file_tag(std::fs::read(path).ok().as_deref()), toml, None)
    }

    /// `toml` and `picture` as a save request expecting `path` as it is now.
    fn req_with(path: &Path, toml: &str, picture: &[u8]) -> Vec<u8> {
        let tag = file_tag(std::fs::read(path).ok().as_deref());
        save_request(&tag, toml, Some(picture))
    }

    /// Where these tests keep the picture: in a directory beside `path`'s.
    fn pic(path: &Path) -> PathBuf {
        path.with_file_name("state").join("background")
    }

    /// A 4x3 picture encoded as `format`.
    fn encoded(format: image::ImageFormat) -> Vec<u8> {
        let mut out = io::Cursor::new(Vec::new());
        image::RgbImage::from_pixel(4, 3, image::Rgb([10, 120, 200]))
            .write_to(&mut out, format)
            .unwrap();
        out.into_inner()
    }

    fn shows_picture() -> String {
        format!("[login]\nbackground = \"{BACKGROUND_PICTURE}\"\n")
    }

    fn pixels(bytes: &[u8]) -> image::RgbImage {
        image::load_from_memory(bytes).unwrap().to_rgb8()
    }

    fn mode(p: &Path) -> u32 {
        std::fs::symlink_metadata(p).unwrap().permissions().mode() & 0o7777
    }

    #[test]
    fn apply_writes_the_parsed_settings_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("login.toml");
        std::fs::write(&path, "[login]\nshow_users = true\n").unwrap();
        let input = "[login]\nshow_users = false\n[remote_desktop]\nenabled = true\nport = 3390\n";
        apply(&req(&path, input), &path, &pic(&path)).unwrap();
        let (config, warnings) = MachineConfig::parse(&std::fs::read_to_string(&path).unwrap());
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(config, MachineConfig::parse(input).0);
        assert!(!config.login.show_users);
        assert_eq!((config.remote.enabled, config.remote.port), (true, 3390));
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o7777, 0o644);
        let mut names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        names.sort();
        assert_eq!(names, [".login.toml.lock", "login.toml"]);
    }

    #[test]
    fn a_planted_tmp_symlink_is_replaced_and_its_target_kept() {
        let dir = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let victim = elsewhere.path().join("victim");
        std::fs::write(&victim, "victim").unwrap();
        std::fs::set_permissions(&victim, Permissions::from_mode(0o600)).unwrap();
        let path = dir.path().join("login.toml");
        std::os::unix::fs::symlink(&victim, dir.path().join("login.toml.tmp")).unwrap();
        apply(
            &req(&path, "[login]\nshow_users = false\n"),
            &path,
            &pic(&path),
        )
        .unwrap();
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "victim");
        let mode = std::fs::metadata(&victim).unwrap().permissions().mode();
        assert_eq!(mode & 0o7777, 0o600);
        assert!(
            !MachineConfig::parse(&std::fs::read_to_string(&path).unwrap())
                .0
                .login
                .show_users
        );
        assert!(std::fs::symlink_metadata(dir.path().join("login.toml.tmp")).is_err());
    }

    const CHILD_DIR: &str = "REPRISE_MACHINEWRITE_DIR";

    #[test]
    #[ignore = "run by apply_sets_the_modes_whatever_the_umask"]
    fn apply_in_a_child() {
        let Some(dir) = std::env::var_os(CHILD_DIR) else {
            return;
        };
        let path = Path::new(&dir).join("login.toml");
        apply(
            &req(&path, "[login]\nshow_users = false\n"),
            &path,
            &pic(&path),
        )
        .unwrap();
    }

    #[test]
    fn apply_sets_the_modes_whatever_the_umask() {
        use std::os::unix::process::CommandExt as _;
        for umask in [0, 0o077] {
            let root = tempfile::tempdir().unwrap();
            let dir = root.path().join("reprise");
            let mut child = std::process::Command::new(std::env::current_exe().unwrap());
            child
                .args([
                    "--exact",
                    "machinewrite::tests::apply_in_a_child",
                    "--ignored",
                ])
                .env(CHILD_DIR, &dir);
            // SAFETY: umask is async-signal-safe and touches only the child.
            unsafe {
                child.pre_exec(move || {
                    libc::umask(umask);
                    Ok(())
                });
            }
            let out = child.output().unwrap();
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stdout)
            );
            let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o7777;
            assert_eq!(mode(&dir), 0o755, "directory under umask {umask:o}");
            assert_eq!(
                mode(&dir.join("login.toml")),
                0o644,
                "file under umask {umask:o}"
            );
        }
    }

    #[test]
    fn a_failed_rename_is_an_io_error_and_leaves_no_tmp() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("login.toml");
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("keep"), "").unwrap();
        let err = apply(
            &save_request("none", "[login]\nshow_users = false\n", None),
            &path,
            &pic(&path),
        )
        .unwrap_err();
        assert!(matches!(err, Error::Io(_)), "{err:?}");
        let mut names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        names.sort();
        assert_eq!(names, [".login.toml.lock", "login.toml"]);
    }

    #[test]
    fn apply_refuses_invalid_settings_and_keeps_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("login.toml");
        let before = "[remote_desktop]\nport = 3390\n";
        std::fs::write(&path, before).unwrap();
        let Err(Error::Invalid(warnings)) = apply(
            &req(&path, "[remote_desktop]\nport = 0\n"),
            &path,
            &pic(&path),
        ) else {
            panic!("port 0 is refused as invalid");
        };
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
    }

    #[test]
    fn unknown_tables_are_dropped_and_known_keys_kept() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("login.toml");
        apply(
            &req(
                &path,
                "[other]\nkey = 1\n[remote_desktop]\ngroup = \"rdp\"\n",
            ),
            &path,
            &pic(&path),
        )
        .unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let table: toml::Table = text.parse().unwrap();
        let mut keys: Vec<_> = table.keys().map(String::as_str).collect();
        keys.sort();
        assert_eq!(keys, ["login", "remote_desktop"]);
        let (config, _) = MachineConfig::parse(&text);
        assert_eq!(
            config.remote,
            MachineRemote {
                group: "rdp".into(),
                ..MachineRemote::default()
            }
        );
    }

    #[test]
    fn a_file_changed_since_it_was_read_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("login.toml");
        std::fs::write(&path, "[remote_desktop]\nenabled = true\n").unwrap();
        let stale = req(&path, "[remote_desktop]\nenabled = true\nport = 3390\n");
        let turned_off = "[remote_desktop]\nenabled = false\n";
        std::fs::write(&path, turned_off).unwrap();
        assert!(matches!(
            apply(&stale, &path, &pic(&path)),
            Err(Error::Changed)
        ));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), turned_off);
        std::fs::remove_file(&path).unwrap();
        assert!(
            matches!(apply(&stale, &path, &pic(&path)), Err(Error::Changed)),
            "a removed file is a change too"
        );
        assert!(!path.exists());
    }

    #[test]
    fn a_request_without_its_tag_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("login.toml");
        let Err(Error::Invalid(w)) = apply(b"[login]\nshow_users = false\n", &path, &pic(&path))
        else {
            panic!("the bare TOML of an older Settings is refused");
        };
        assert_eq!(w, ["the settings do not say which file they replace"]);
        assert!(!path.exists());
    }

    #[test]
    fn a_save_waits_for_the_one_holding_the_lock() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("login.toml");
        let held = lock(&path, Duration::ZERO).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let p = path.clone();
        std::thread::spawn(move || {
            let _ = tx.send(apply(&req(&p, "[login]\nshow_users = false\n"), &p, &pic(&p)).is_ok());
        });
        assert!(
            rx.recv_timeout(std::time::Duration::from_millis(300))
                .is_err(),
            "the save waits"
        );
        drop(held);
        assert_eq!(rx.recv_timeout(std::time::Duration::from_secs(5)), Ok(true));
    }

    #[test]
    fn a_lock_on_the_directory_does_not_hold_up_a_save() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("login.toml");
        let d = File::open(dir.path()).unwrap();
        // SAFETY: flock on a descriptor the test owns.
        assert_eq!(unsafe { libc::flock(d.as_raw_fd(), libc::LOCK_EX) }, 0);
        let (tx, rx) = std::sync::mpsc::channel();
        let p = path.clone();
        std::thread::spawn(move || {
            let _ = tx.send(apply(&req(&p, "[login]\nshow_users = false\n"), &p, &pic(&p)).is_ok());
        });
        assert_eq!(rx.recv_timeout(Duration::from_secs(3)), Ok(true));
        let mode = std::fs::metadata(lock_path(&path))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o7777, 0o600, "only root opens the lock");
    }

    #[test]
    fn a_save_held_up_past_its_wait_gives_up() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("login.toml");
        let _held = lock(&path, Duration::ZERO).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let p = path.clone();
        std::thread::spawn(move || {
            let r = apply_waiting(
                &req(&p, "[login]\nshow_users = false\n"),
                &p,
                &pic(&p),
                Duration::from_millis(200),
            );
            let _ = tx.send(matches!(r, Err(Error::Busy)));
        });
        assert_eq!(rx.recv_timeout(Duration::from_secs(3)), Ok(true));
        assert!(!path.exists());
    }

    #[test]
    fn a_picture_is_copied_readable_by_all_and_the_settings_name_it() {
        for format in [
            image::ImageFormat::Png,
            image::ImageFormat::Jpeg,
            image::ImageFormat::Bmp,
        ] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("login.toml");
            let picture = encoded(format);
            apply(
                &req_with(&path, &shows_picture(), &picture),
                &path,
                &pic(&path),
            )
            .unwrap();
            let stored = std::fs::read(pic(&path)).unwrap();
            assert_eq!(
                image::guess_format(&stored).unwrap(),
                image::ImageFormat::Png,
                "{format:?}"
            );
            assert_eq!(pixels(&stored), pixels(&picture), "{format:?}");
            assert_eq!(mode(&pic(&path)), 0o644, "{format:?}");
            assert_eq!(mode(pic(&path).parent().unwrap()), 0o711, "{format:?}");
            let (config, _) = MachineConfig::parse(&std::fs::read_to_string(&path).unwrap());
            assert_eq!(config.login.background, Some(Background::Picture));
        }
    }

    #[test]
    fn a_picture_dir_shut_to_others_is_opened_for_passing_through_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("login.toml");
        let state = pic(&path).parent().unwrap().to_owned();
        std::fs::create_dir(&state).unwrap();
        std::fs::set_permissions(&state, Permissions::from_mode(0o700)).unwrap();
        let png = encoded(image::ImageFormat::Png);
        apply(&req_with(&path, &shows_picture(), &png), &path, &pic(&path)).unwrap();
        assert_eq!(mode(&state), 0o711);
    }

    #[test]
    fn an_oversize_picture_is_refused_and_nothing_written() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("login.toml");
        let mut huge = encoded(image::ImageFormat::Png);
        huge.resize(MAX_PICTURE + 1, 0);
        let Err(Error::Invalid(w)) = apply(
            &req_with(&path, &shows_picture(), &huge),
            &path,
            &pic(&path),
        ) else {
            panic!("a picture past {MAX_PICTURE} bytes is refused");
        };
        assert_eq!(w, ["the picture is over 32 MB"]);
        assert!(!path.exists() && !pic(&path).exists());
    }

    #[test]
    fn a_file_that_is_no_picture_the_screen_can_show_is_refused_saying_why() {
        let png = encoded(image::ImageFormat::Png);
        let not_one = "the picture is not a PNG, JPEG or BMP image the login screen can show";
        let gif = b"GIF89a\x04\x00\x03\x00\x00\x00\x00;".to_vec();
        let mut wide = io::Cursor::new(Vec::new());
        (image::GrayImage::new(16385, 1).write_to(&mut wide, image::ImageFormat::Png)).unwrap();
        for (bad, why) in [
            (b"[login]\nshow_users = false\n".to_vec(), not_one),
            (gif, not_one),
            (Vec::new(), not_one),
            (
                png[..png.len() - 20].to_vec(),
                "the picture is damaged or cut short",
            ),
            (
                wide.into_inner(),
                "the picture is wider or taller than 16384 pixels",
            ),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("login.toml");
            let Err(Error::Invalid(w)) =
                apply(&req_with(&path, &shows_picture(), &bad), &path, &pic(&path))
            else {
                panic!("{why}: refused");
            };
            assert_eq!(w, [why]);
            assert!(!path.exists() && !pic(&path).exists());
        }
    }

    /// A PNG signature, an RGBA IHDR and an empty IDAT: a size with no pixels behind it.
    pub(crate) fn png_header(w: u32, h: u32) -> Vec<u8> {
        let chunk = |png: &mut Vec<u8>, kind: &[u8], data: &[u8]| {
            let body = [kind, data].concat();
            let mut crc = !0u32;
            for &b in &body {
                crc ^= u32::from(b);
                for _ in 0..8 {
                    crc = (crc >> 1) ^ (0xEDB8_8320 & (crc & 1).wrapping_neg());
                }
            }
            png.extend((data.len() as u32).to_be_bytes());
            png.extend(body);
            png.extend((!crc).to_be_bytes());
        };
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        let ihdr = [&w.to_be_bytes()[..], &h.to_be_bytes(), &[8, 6, 0, 0, 0]].concat();
        chunk(&mut png, b"IHDR", &ihdr);
        chunk(&mut png, b"IDAT", &[]);
        png
    }

    #[test]
    fn a_picture_past_the_pixel_cap_is_refused_by_its_header() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("login.toml");
        for ((w, h), why) in [
            ((8000, 8000), "the picture is damaged or cut short"),
            ((8001, 8000), "the picture has more than 64 million pixels"),
        ] {
            let Err(Error::Invalid(got)) = apply(
                &req_with(&path, &shows_picture(), &png_header(w, h)),
                &path,
                &pic(&path),
            ) else {
                panic!("{w}x{h}: refused");
            };
            assert_eq!(got, [why], "{w}x{h}");
            assert!(!path.exists() && !pic(&path).exists());
        }
    }

    #[test]
    fn the_stored_size_caps_pixels_and_keeps_the_shape() {
        assert_eq!(stored_size(4000, 3000), (4000, 3000));
        assert_eq!(
            stored_size(16000, 2000),
            (16000, 2000),
            "a panorama keeps its pixels"
        );
        assert_eq!(stored_size(15360, 8640), (7680, 4320));
        assert_eq!(stored_size(6048, 8064), (4988, 6651));
        let (w, h) = stored_size(16384, 10900);
        assert!(u64::from(w) * u64::from(h) <= STORED_PIXELS, "{w}x{h}");
        assert_eq!((w as f64 / h as f64 * 100.0).round(), 150.0, "{w}x{h}");
    }

    #[test]
    fn a_picture_scaled_down_shows_no_aliasing() {
        let stripes =
            image::RgbImage::from_fn(1400, 2, |x, _| image::Rgb([(x % 2) as u8 * 255; 3]));
        let small = shrink(stripes, 1000, 2);
        assert_eq!(small.dimensions(), (1000, 2));
        for (x, y, p) in small.enumerate_pixels() {
            assert!(
                (64..=192).contains(&p.0[0]),
                "({x}, {y}) is {:?}, not grey",
                p.0
            );
        }
        let flat = shrink(
            image::RgbImage::from_pixel(700, 300, image::Rgb([10, 120, 200])),
            500,
            200,
        );
        assert!(flat.pixels().all(|p| p.0 == [10, 120, 200]));
    }

    /// An 8x4 JPEG, left half red and right half blue, tagged Exif orientation 6: shown turned a
    /// quarter clockwise.
    #[test]
    fn a_photo_is_stored_the_way_up_its_orientation_says() {
        use image::ImageEncoder as _;
        let img = image::RgbImage::from_fn(8, 4, |x, _| {
            image::Rgb(if x < 4 { [255, 0, 0] } else { [0, 0, 255] })
        });
        let exif = [
            b'M', b'M', 0, 42, 0, 0, 0, 8, 0, 1, 0x01, 0x12, 0, 3, 0, 0, 0, 1, 0, 6, 0, 0, 0, 0, 0,
            0,
        ];
        let mut photo = Vec::new();
        let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut photo, 100);
        enc.set_exif_metadata(exif.to_vec()).unwrap();
        (enc.write_image(img.as_raw(), 8, 4, image::ExtendedColorType::Rgb8)).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("login.toml");
        apply(
            &req_with(&path, &shows_picture(), &photo),
            &path,
            &pic(&path),
        )
        .unwrap();
        let stored = pixels(&std::fs::read(pic(&path)).unwrap());
        assert_eq!(stored.dimensions(), (4, 8));
        let [r, _, b] = stored.get_pixel(1, 1).0;
        assert!(r > 200 && b < 60, "red on top");
        let [r, _, b] = stored.get_pixel(1, 6).0;
        assert!(r < 60 && b > 200, "blue below");
    }

    #[test]
    fn a_planted_picture_symlink_is_replaced_and_its_target_kept() {
        let dir = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let victim = elsewhere.path().join("victim");
        std::fs::write(&victim, "victim").unwrap();
        let path = dir.path().join("login.toml");
        std::fs::create_dir(pic(&path).parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&victim, pic(&path)).unwrap();
        std::os::unix::fs::symlink(&victim, pic(&path).with_file_name("background.tmp")).unwrap();
        let png = encoded(image::ImageFormat::Png);
        apply(&req_with(&path, &shows_picture(), &png), &path, &pic(&path)).unwrap();
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "victim");
        assert!(std::fs::symlink_metadata(pic(&path)).unwrap().is_file());
        assert_eq!(pixels(&std::fs::read(pic(&path)).unwrap()), pixels(&png));
    }

    #[test]
    fn the_stored_picture_keeps_none_of_the_photos_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("login.toml");
        let jpeg = encoded(image::ImageFormat::Jpeg);
        let exif = b"Exif\0\0MM\0*GPS 47.37N 8.54E serial 0451";
        let mut photo = jpeg[..2].to_vec();
        photo.extend([0xff, 0xe1]);
        photo.extend(((exif.len() + 2) as u16).to_be_bytes());
        photo.extend(exif);
        photo.extend(&jpeg[2..]);
        apply(
            &req_with(&path, &shows_picture(), &photo),
            &path,
            &pic(&path),
        )
        .unwrap();
        let stored = std::fs::read(pic(&path)).unwrap();
        assert!(
            !stored.windows(12).any(|w| w == b"47.37N 8.54E"),
            "the EXIF block stays behind"
        );
        assert_eq!(pixels(&stored), pixels(&jpeg));
    }

    #[test]
    fn a_picture_waits_for_the_settings_and_goes_with_them_when_they_fail() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("login.toml");
        let blocked = dir.path().join("login.toml.tmp");
        std::fs::create_dir(&blocked).unwrap();
        std::fs::write(blocked.join("keep"), "").unwrap();
        let png = encoded(image::ImageFormat::Png);
        let err = apply(&req_with(&path, &shows_picture(), &png), &path, &pic(&path)).unwrap_err();
        assert!(matches!(err, Error::Io(_)), "{err:?}");
        assert!(!pic(&path).exists(), "no picture without its settings");
        assert!(!pic(&path).with_file_name("background.tmp").exists());
    }

    #[test]
    fn settings_showing_no_picture_remove_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("login.toml");
        let png = encoded(image::ImageFormat::Png);
        apply(&req_with(&path, &shows_picture(), &png), &path, &pic(&path)).unwrap();
        apply(&req(&path, &shows_picture()), &path, &pic(&path)).unwrap();
        assert!(
            pic(&path).exists(),
            "a save without a picture keeps the one there"
        );
        let builtin = "[login]\nbackground = \"reprise:tarn/dusk\"\n";
        apply(&req(&path, builtin), &path, &pic(&path)).unwrap();
        assert!(!pic(&path).exists());
    }

    #[test]
    fn a_picture_the_settings_do_not_show_or_settings_without_one_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("login.toml");
        let png = encoded(image::ImageFormat::Png);
        let Err(Error::Invalid(w)) = apply(
            &req_with(&path, "[login]\nshow_users = false\n", &png),
            &path,
            &pic(&path),
        ) else {
            panic!("a picture for settings that show none is refused");
        };
        assert_eq!(w, ["a picture came with settings that do not show it"]);
        let Err(Error::Invalid(w)) = apply(&req(&path, &shows_picture()), &path, &pic(&path))
        else {
            panic!("settings naming a picture never sent are refused");
        };
        assert_eq!(w, ["there is no picture for the background; pick one"]);
        assert!(!path.exists() && !pic(&path).exists());
    }
}
