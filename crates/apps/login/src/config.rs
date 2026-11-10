use std::os::unix::fs::OpenOptionsExt as _;
use std::path::Path;

use repriselogin::machine::{BACKGROUND_PICTURE, Background, MachineConfig};

use crate::machinewrite::{MAX_STORED, stored_size};

/// Reads the machine settings at `path`, logging each warning. A missing file gives the defaults.
pub fn load(path: &Path) -> MachineConfig {
    load_with(path, Path::new(BACKGROUND_PICTURE))
}

/// [`load`], the settings' picture at `picture`: one the login screen can't show is dropped, so
/// the greeting names no background.
fn load_with(path: &Path, picture: &Path) -> MachineConfig {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return MachineConfig::default(),
        Err(e) => {
            log::warn!("{}: {e}", path.display());
            return MachineConfig::default();
        }
    };
    let (mut config, warnings) = MachineConfig::parse(&text);
    for w in warnings {
        log::warn!("{}: {w}", path.display());
    }
    if config.login.background == Some(Background::Picture)
        && let Err(why) = shown(picture)
    {
        log::warn!(
            "{}: {why}; the login screen shows its default",
            picture.display()
        );
        config.login.background = None;
    }
    config
}

/// Why `picture` is no file the login screen can show, if it is not. Only its header is read:
/// root wrote it, and the greeter falls back when the rest does not decode.
fn shown(picture: &Path) -> Result<(), String> {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(picture)
        .map_err(|e| e.to_string())?;
    let meta = file.metadata().map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err("not a file".into());
    }
    if meta.len() > MAX_STORED {
        return Err(format!("over {} MB", MAX_STORED >> 20));
    }
    let reader = image::ImageReader::new(std::io::BufReader::new(file))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    if reader.format() != Some(image::ImageFormat::Png) {
        return Err("not a PNG".into());
    }
    let (w, h) = reader.into_dimensions().map_err(|e| e.to_string())?;
    if stored_size(w, h) != (w, h) {
        return Err(format!("{w}x{h} is larger than the helper stores"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_the_login_screen_cannot_show_is_left_out() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("login.toml");
        std::fs::write(
            &path,
            format!("[login]\nbackground = \"{BACKGROUND_PICTURE}\"\n"),
        )
        .unwrap();
        let picture = dir.path().join("background");
        let background = || load_with(&path, &picture).login.background;
        assert_eq!(background(), None, "missing");
        std::fs::write(&picture, b"\x89PNG\r\n\x1a\n cut short").unwrap();
        assert_eq!(background(), None, "undecodable");
        std::fs::remove_file(&picture).unwrap();
        let mut png = std::io::Cursor::new(Vec::new());
        (image::RgbImage::from_pixel(4, 3, image::Rgb([1, 2, 3])))
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        let real = dir.path().join("real.png");
        std::fs::write(&real, png.get_ref()).unwrap();
        std::os::unix::fs::symlink(&real, &picture).unwrap();
        assert_eq!(background(), None, "a symlink");
        std::fs::remove_file(&picture).unwrap();
        std::fs::write(&picture, png.get_ref()).unwrap();
        assert_eq!(background(), Some(Background::Picture));
    }

    /// `w` x `h` RGBA pixels of noise as a PNG, which compresses to about its raw size.
    fn noise_png(w: u32, h: u32) -> Vec<u8> {
        let mut seed = 0x2545_f491_u32;
        let noise = image::RgbaImage::from_fn(w, h, |_, _| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            image::Rgba(seed.to_le_bytes())
        });
        let mut png = std::io::Cursor::new(Vec::new());
        noise.write_to(&mut png, image::ImageFormat::Png).unwrap();
        png.into_inner()
    }

    #[test]
    fn a_stored_picture_is_shown_by_its_header_alone_up_to_the_stored_size() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("login.toml");
        std::fs::write(
            &path,
            format!("[login]\nbackground = \"{BACKGROUND_PICTURE}\"\n"),
        )
        .unwrap();
        let picture = dir.path().join("background");
        let background = || load_with(&path, &picture).login.background;
        let big = noise_png(3000, 3000);
        assert!(big.len() > repriselogin::machine::MAX_PICTURE);
        std::fs::write(&picture, &big).unwrap();
        assert_eq!(
            background(),
            Some(Background::Picture),
            "past the upload cap"
        );
        std::fs::write(&picture, &big[..big.len() / 2]).unwrap();
        assert_eq!(
            background(),
            Some(Background::Picture),
            "the body is the greeter's to decode"
        );
        let png_header = crate::machinewrite::tests::png_header;
        std::fs::write(&picture, png_header(16000, 2000)).unwrap();
        assert_eq!(background(), Some(Background::Picture), "a panorama");
        std::fs::write(&picture, png_header(7681, 4320)).unwrap();
        assert_eq!(background(), None, "more pixels than the helper stores");
        let mut jpeg = std::io::Cursor::new(Vec::new());
        (image::RgbImage::new(4, 3).write_to(&mut jpeg, image::ImageFormat::Jpeg)).unwrap();
        std::fs::write(&picture, jpeg.get_ref()).unwrap();
        assert_eq!(background(), None, "the helper stores only PNG");
    }

    #[test]
    fn a_missing_file_gives_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let config = load(&dir.path().join("login.toml"));
        assert_eq!(config, MachineConfig::default());
        assert!(config.login.show_users);
        assert!(
            !config.login.greeter_socket,
            "the login screen has no socket"
        );
        assert!(!config.remote.enabled, "remote logins ship off");
    }
}
