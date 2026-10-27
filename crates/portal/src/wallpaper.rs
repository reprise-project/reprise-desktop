//! `org.freedesktop.impl.portal.Wallpaper`: an app's picture as the
//! desktop background, after the Wallpaper permit prompt. The picture is
//! copied under `$XDG_DATA_HOME/reprise/wallpapers`, so a document portal
//! path that goes away with the app does not take the background with it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use toml_edit::{DocumentMut, Value};
use zbus::zvariant::{self, ObjectPath};

const IMAGE: repriseconf::Key = repriseconf::key("wallpaper", "image");
const SLIDESHOW: repriseconf::Key = repriseconf::key("wallpaper", "slideshow");
const LOCK_SCREEN: repriseconf::Key = repriseconf::key("wallpaper", "lock_screen");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SetOn {
    Background,
    Both,
}

pub struct Wallpaper {
    pub frontend: crate::Frontend,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Wallpaper")]
impl Wallpaper {
    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        1
    }

    #[allow(clippy::too_many_arguments)]
    async fn set_wallpaper_uri(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        handle: ObjectPath<'_>,
        app_id: String,
        parent_window: String,
        uri: String,
        options: HashMap<String, zvariant::Value<'_>>,
        #[zbus(object_server)] server: &zbus::ObjectServer,
    ) -> zbus::fdo::Result<u32> {
        self.frontend.check(&hdr).await?;
        let set_on = match options.get("set-on") {
            None => Some(SetOn::Background),
            Some(zvariant::Value::Str(s)) => set_on(s.as_str()),
            Some(_) => None,
        };
        let Some(set_on) = set_on else {
            log::info!("SetWallpaperURI for {app_id:?}: no lock screen picture of its own");
            return Ok(crate::RESPONSE_OTHER);
        };
        let Some(path) = file_path(&uri) else {
            log::info!("SetWallpaperURI for {app_id:?}: {uri:?} is not a local file");
            return Ok(crate::RESPONSE_OTHER);
        };
        let show_preview = matches!(
            options.get("show-preview"),
            Some(zvariant::Value::Bool(true))
        );
        let src = path.clone();
        let Some(bytes) = blocking::unblock(move || read_picture(&src)).await else {
            log::info!(
                "SetWallpaperURI for {app_id:?}: {} is no picture",
                path.display()
            );
            return Ok(crate::RESPONSE_OTHER);
        };
        let cancel = Arc::new(AtomicBool::new(false));
        crate::export_request(server, &handle, cancel.clone(), &self.frontend).await?;
        let input = crate::permit::Input {
            parent: crate::dialog::parent_handle(&parent_window),
            app: String::new(),
            offer_remember: false,
            items: vec![file_name(&path)],
            preview: shows_preview(&app_id, show_preview)
                .then(|| path.to_string_lossy().into_owned()),
        };
        let id = app_id.clone();
        let allowed = blocking::unblock(move || {
            let input = crate::permit::Input {
                app: crate::remotedesktop::display_name(&id),
                ..input
            };
            let kind = crate::permit::Kind::Wallpaper {
                lock_screen: set_on == SetOn::Both,
            };
            crate::permit::ask_with(kind, input, &cancel).allowed()
        })
        .await;
        let _ = server.remove::<crate::Request, _>(handle.clone()).await;
        if !allowed {
            log::info!("SetWallpaperURI for {app_id:?} denied");
            return Ok(crate::RESPONSE_CANCELLED);
        }
        let name = file_name(&path);
        let dir = repriseconf::xdg_home("XDG_DATA_HOME", ".local/share").join("reprise/wallpapers");
        let config = repriseconf::config_path();
        match blocking::unblock(move || install(&dir, &name, &bytes, set_on, &config)).await {
            Ok(to) => {
                log::info!("SetWallpaperURI for {app_id:?}: {}", to.display());
                Ok(crate::RESPONSE_OK)
            }
            Err(e) => {
                log::warn!("SetWallpaperURI for {app_id:?}: {e}");
                Ok(crate::RESPONSE_OTHER)
            }
        }
    }
}

/// Where `set-on` puts the picture; None for the lock screen alone, which
/// shows the desktop's picture or none.
fn set_on(s: &str) -> Option<SetOn> {
    match s {
        "background" => Some(SetOn::Background),
        "both" => Some(SetOn::Both),
        _ => None,
    }
}

/// The local path of a `file://` URI. The frontend turns a file descriptor into the host path
/// it names, so a sandboxed app can name any file the user can read.
fn file_path(uri: &str) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    let rest = uri.strip_prefix("file://")?;
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    if !rest.starts_with('/') || rest.contains(['?', '#']) {
        return None;
    }
    let bytes = reprisefs::trash::percent_decode(rest);
    if bytes.contains(&0) {
        return None;
    }
    Some(PathBuf::from(std::ffi::OsString::from_vec(bytes)))
}

/// Whether the prompt shows the picture: always for a sandboxed app, which may have picked any
/// picture the user owns; an unsandboxed one asks with `show-preview`.
fn shows_preview(app_id: &str, asked: bool) -> bool {
    asked || !app_id.is_empty()
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| crate::dialog::one_line(&n.to_string_lossy()))
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| "Wallpaper".into())
}

/// The bytes of the regular file at `path` when its header names a picture the desktop shows,
/// no side past [`reprisekit::wallpaper::MAX_SIDE`].
fn read_picture(path: &Path) -> Option<Vec<u8>> {
    use std::os::unix::fs::OpenOptionsExt;
    let f = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOCTTY)
        .open(path)
        .ok()?;
    if !f.metadata().ok()?.file_type().is_file() {
        return None;
    }
    let bytes = repriseapp::decode::read_capped_from(f).ok()?;
    let (w, h) = image::ImageReader::new(std::io::Cursor::new(&bytes))
        .with_guessed_format()
        .ok()?
        .into_dimensions()
        .ok()?;
    let max = reprisekit::wallpaper::MAX_SIDE;
    (w <= max && h <= max).then_some(bytes)
}

/// Write `bytes` as `name` in `dir`, point `[wallpaper]` in `config` at the copy and delete the
/// copies earlier calls left in `dir`.
fn install(
    dir: &Path,
    name: &str,
    bytes: &[u8],
    set_on: SetOn,
    config: &Path,
) -> Result<PathBuf, String> {
    use std::io::Write;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let (mut file, to) =
        reprisefs::ops::create_new_numbered(dir, name).map_err(|e| e.to_string())?;
    let written = file
        .write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|e| e.to_string())
        .and_then(|()| {
            repriseconf::edit(config, |text| {
                let mut doc: DocumentMut = text.parse().map_err(|e| format!("{e}"))?;
                apply(&mut doc, &to, set_on)?;
                Ok::<_, String>(doc.to_string())
            })
            .map_err(|e| e.to_string())?
            .map(drop)
        });
    if let Err(e) = written {
        let _ = std::fs::remove_file(&to);
        return Err(e);
    }
    for old in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let file = old.file_type().is_ok_and(|t| t.is_file());
        if file && old.path() != to {
            let _ = std::fs::remove_file(old.path());
        }
    }
    Ok(to)
}

/// Set `image` as the picture, ending a slideshow, which would show in its
/// place; `Both` also puts it behind the lock screen.
fn apply(doc: &mut DocumentMut, image: &Path, set_on: SetOn) -> Result<(), String> {
    let image = image.to_str().ok_or("the picture's path is not UTF-8")?;
    repriseconf::set(doc, &IMAGE, Value::from(image)).map_err(|e| e.to_string())?;
    repriseconf::remove(doc, &SLIDESHOW);
    if set_on == SetOn::Both {
        repriseconf::set(doc, &LOCK_SCREEN, Value::from(true)).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_uri_names_its_local_path_and_others_are_refused() {
        assert_eq!(
            file_path("file:///home/u/Pictures/dunes%20at%20dusk.jpg"),
            Some(PathBuf::from("/home/u/Pictures/dunes at dusk.jpg"))
        );
        assert_eq!(
            file_path("file://localhost/srv/a.png"),
            Some(PathBuf::from("/srv/a.png"))
        );
        assert_eq!(
            file_path("file:///run/user/1000/doc/3f2a91c0/sunrise%2Ejpg"),
            Some(PathBuf::from("/run/user/1000/doc/3f2a91c0/sunrise.jpg"))
        );
        for bad in [
            "https://example.org/a.jpg",
            "file://host/a.jpg",
            "file:///a.jpg?x",
            "file:///a%00b.jpg",
            "/a.jpg",
        ] {
            assert_eq!(file_path(bad), None, "{bad}");
        }
    }

    #[test]
    fn set_on_takes_background_and_both_but_not_the_lock_screen_alone() {
        assert_eq!(set_on("background"), Some(SetOn::Background));
        assert_eq!(set_on("both"), Some(SetOn::Both));
        assert_eq!(set_on("lockscreen"), None);
        assert_eq!(set_on("elsewhere"), None);
    }

    #[test]
    fn the_picture_replaces_a_slideshow_and_both_turns_the_lock_screen_picture_on() {
        let mut doc: DocumentMut = "# mine\n[wallpaper]\nslideshow = \"~/Pictures\"\n\
                                    position = \"center\"\nlock_screen = false\n"
            .parse()
            .unwrap();
        let pic = Path::new("/home/u/.local/share/reprise/wallpapers/dunes.jpg");
        apply(&mut doc, pic, SetOn::Background).unwrap();
        let cfg = repriseconf::config::Config::parse(&doc.to_string()).unwrap();
        assert_eq!(cfg.wallpaper.image.as_deref(), Some(pic));
        assert_eq!(cfg.wallpaper.slideshow, None);
        assert!(!cfg.wallpaper.lock_screen);
        assert!(doc.to_string().starts_with("# mine\n"));
        assert!(doc.to_string().contains("position = \"center\""));
        apply(&mut doc, pic, SetOn::Both).unwrap();
        let cfg = repriseconf::config::Config::parse(&doc.to_string()).unwrap();
        assert!(cfg.wallpaper.lock_screen);
    }

    #[test]
    fn only_a_regular_file_that_decodes_is_a_picture() {
        let dir = tempfile::tempdir().unwrap();
        let png = dir.path().join("dot.png");
        let mut bytes = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut bytes, 1, 1);
            enc.set_color(png::ColorType::Rgb);
            let mut w = enc.write_header().unwrap();
            w.write_image_data(&[1, 2, 3]).unwrap();
        }
        std::fs::write(&png, &bytes).unwrap();
        assert_eq!(read_picture(&png), Some(bytes.clone()));
        let text = dir.path().join("note.png");
        std::fs::write(&text, "not a picture").unwrap();
        assert_eq!(read_picture(&text), None);
        assert_eq!(read_picture(dir.path()), None);
        let to = dir.path().join("wallpapers");
        assert_eq!(file_name(&png), "dot.png");
        let config = dir.path().join("config.toml");
        let copied = install(&to, "dot.png", &bytes, SetOn::Background, &config).unwrap();
        assert_eq!(copied, to.join("dot.png"));
        assert_eq!(std::fs::read(&copied).unwrap(), bytes);
        let written = std::fs::read_to_string(&config).unwrap();
        assert!(
            written.contains(&format!("image = \"{}\"", copied.display())),
            "{written}"
        );
    }

    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut enc = png::Encoder::new(&mut bytes, w, h);
        enc.set_color(png::ColorType::Grayscale);
        let mut out = enc.write_header().unwrap();
        out.write_image_data(&vec![0; (w * h) as usize]).unwrap();
        out.finish().unwrap();
        bytes
    }

    #[test]
    fn a_picture_wider_than_the_desktop_shows_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let max = reprisekit::wallpaper::MAX_SIDE;
        let wide = dir.path().join("wide.png");
        std::fs::write(&wide, png(max + 1, 1)).unwrap();
        assert_eq!(read_picture(&wide), None);
        let widest = dir.path().join("widest.png");
        std::fs::write(&widest, png(max, 1)).unwrap();
        assert!(read_picture(&widest).is_some());
    }

    #[test]
    fn a_new_wallpaper_deletes_the_copies_before_it() {
        let dir = tempfile::tempdir().unwrap();
        let to = dir.path().join("wallpapers");
        let config = dir.path().join("config.toml");
        let bytes = png(1, 1);
        let first = install(&to, "a.png", &bytes, SetOn::Background, &config).unwrap();
        let again = install(&to, "a.png", &bytes, SetOn::Background, &config).unwrap();
        assert_ne!(first, again);
        let last = install(&to, "b.png", &bytes, SetOn::Background, &config).unwrap();
        let left: Vec<PathBuf> = std::fs::read_dir(&to)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        assert_eq!(left, [last]);
    }

    #[test]
    fn a_sandboxed_apps_picture_is_always_shown_in_the_prompt() {
        assert!(shows_preview("org.example.Photos", false));
        assert!(shows_preview("", true));
        assert!(!shows_preview("", false));
    }
}
