//! `org.freedesktop.impl.portal.Wallpaper`: an app's picture as the
//! desktop background, after the Wallpaper permit prompt or an Allow the
//! permission store remembers for a picture the app reads itself. The picture is copied under
//! `$XDG_DATA_HOME/reprise/wallpapers`, so a document portal path that goes
//! away with the app does not take the background with it.

use reprisekit::tr;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use toml_edit::{DocumentMut, Value};
use zbus::ObjectPath;

const IMAGE: repriseconf::Key = repriseconf::key("wallpaper", "image");
const SLIDESHOW: repriseconf::Key = repriseconf::key("wallpaper", "slideshow");

/// xdg-desktop-portal's own entry. It denies a `no` itself, and an answer to its own prompt (a call
/// without `show-preview`) is written here before the call arrives, so a `yes` may never have seen
/// a picture: see [`remembered_applies`].
const STORE: crate::permit::Store = crate::permit::Store {
    table: "wallpaper",
    id: "wallpaper",
};

pub struct Wallpaper {
    pub frontend: crate::Frontend,
    pub read: Read,
    pub open: Open,
}

/// The app's picture in a sealed memfd and the path its file really is, [`read_picture`] outside
/// tests.
pub type Read = fn(&Path) -> Option<(File, PathBuf)>;

/// The most wallpaper requests open at once, each holding up to
/// [`reprisekit::wallpaper::MAX_BYTES`] of picture from its read to its install.
const MAX_OPEN: usize = 4;

/// The app ids with a wallpaper request open. An app has one at a time.
#[derive(Clone, Default)]
pub struct Open(Arc<Mutex<HashSet<String>>>);

impl Open {
    /// `app_id`'s turn until it drops; None while one of its requests is open or [`MAX_OPEN`] are.
    fn take(&self, app_id: &str) -> Option<Turn> {
        let mut open = reprisebus::mutex::lock(&self.0);
        (open.len() < MAX_OPEN && open.insert(app_id.to_owned())).then(|| Turn {
            open: self.clone(),
            app_id: app_id.to_owned(),
        })
    }
}

struct Turn {
    open: Open,
    app_id: String,
}

impl Drop for Turn {
    fn drop(&mut self) {
        reprisebus::mutex::lock(&self.open.0).remove(&self.app_id);
    }
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Wallpaper")]
impl Wallpaper {
    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        1
    }

    #[allow(clippy::too_many_arguments)]
    #[zbus(name = "SetWallpaperURI")]
    async fn set_wallpaper_uri(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        handle: ObjectPath<'_>,
        app_id: String,
        parent_window: String,
        uri: String,
        options: HashMap<String, zbus::Variant<'_>>,
        #[zbus(object_server)] server: &zbus::ObjectServer,
    ) -> zbus::fdo::Result<u32> {
        let asker = self.frontend.check(&hdr).await?;
        let request = crate::Served::new(server, &handle, &asker).await?;
        let code = self
            .set(&request, app_id, parent_window, uri, &options)
            .await;
        request.end().await;
        Ok(code)
    }
}

impl Wallpaper {
    async fn set(
        &self,
        request: &crate::Served<'_>,
        app_id: String,
        parent_window: String,
        uri: String,
        options: &HashMap<String, zbus::Variant<'_>>,
    ) -> u32 {
        let takes = match options.get("set-on") {
            None => true,
            Some(zbus::Variant::Str(s)) => sets_the_desktop(s.as_str()),
            Some(_) => false,
        };
        if !takes {
            log::info!("SetWallpaperURI for {app_id:?}: no lock screen picture of its own");
            return crate::RESPONSE_OTHER;
        };
        let Some(path) = file_path(&uri) else {
            log::info!("SetWallpaperURI for {app_id:?}: {uri:?} is not a local file");
            return crate::RESPONSE_OTHER;
        };
        let Some(_turn) = self.open.take(&app_id) else {
            log::info!("SetWallpaperURI for {app_id:?}: refused while another request is open");
            return crate::RESPONSE_OTHER;
        };
        let show_preview = matches!(options.get("show-preview"), Some(zbus::Variant::Bool(true)));
        let (src, read) = (path.clone(), self.read);
        let Some((picture, real)) = blocking::unblock(move || read(&src)).await else {
            log::info!(
                "SetWallpaperURI for {app_id:?}: {} is no picture",
                path.display()
            );
            return crate::RESPONSE_OTHER;
        };
        let parent = crate::dialog::parent_handle(&parent_window);
        let items = vec![file_name(&path)];
        let shown = shows_preview(&app_id, show_preview);
        let id = app_id.clone();
        let asked = request.ask(None, move |cancel| {
            // The prompt decodes the memfd read above, so the picture shown is the one
            // installed whatever the app's file holds by the time the prompt reads it.
            let preview = shown.then(|| fd_path(&picture));
            let input = crate::permit::Input {
                parent,
                app: crate::remotedesktop::display_name(&id),
                items,
                preview,
                ..Default::default()
            };
            let kind = crate::permit::Kind::Wallpaper;
            let allowed = match remembered_applies(&real, &app_folders(&id)) {
                true => crate::permit::ask_remembered(kind, STORE, &id, &[], input, &cancel),
                false => crate::permit::ask_with(kind, input, &cancel).allowed(),
            };
            (allowed, picture)
        });
        let Some((true, picture)) = asked.await.filter(|_| !request.closed()) else {
            log::info!("SetWallpaperURI for {app_id:?} denied or closed");
            return crate::RESPONSE_CANCELLED;
        };
        let name = file_name(&path);
        let dir = reprisexdg::data_home()
            .unwrap_or_default()
            .join("reprise/wallpapers");
        let config = repriseconf::config_path();
        match blocking::unblock(move || install(&dir, &name, &picture, &config)).await {
            Ok(to) => {
                log::info!("SetWallpaperURI for {app_id:?}: {}", to.display());
                crate::RESPONSE_OK
            }
            Err(e) => {
                log::warn!("SetWallpaperURI for {app_id:?}: {e}");
                crate::RESPONSE_OTHER
            }
        }
    }
}

/// `from`, read to its end, in a sealed memfd: the one copy of the picture the portal holds.
/// Refused once it passes `cap` bytes, without reading further.
fn sealed_copy(from: impl std::io::Read, cap: u64) -> std::io::Result<File> {
    use std::os::fd::{AsRawFd, FromRawFd};
    let flags = libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING;
    // SAFETY: the name is a NUL-terminated string that outlives the call.
    let fd = unsafe { libc::memfd_create(c"reprise-wallpaper".as_ptr(), flags) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: `fd` is a fresh descriptor that nothing else owns.
    let mut file = unsafe { File::from_raw_fd(fd) };
    // From a file, std copies in the kernel (copy_file_range, sendfile), through no buffer here.
    if std::io::copy(&mut from.take(cap + 1), &mut file)? > cap {
        return Err(std::io::ErrorKind::FileTooLarge.into());
    }
    let seals = libc::F_SEAL_WRITE | libc::F_SEAL_SHRINK | libc::F_SEAL_GROW | libc::F_SEAL_SEAL;
    // SAFETY: `file` is an open memfd; F_ADD_SEALS takes an int argument.
    if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_ADD_SEALS, seals) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(file)
}

/// The path another process of the user opens `file` by, at offset 0 of its own.
fn fd_path(file: &File) -> String {
    use std::os::fd::AsRawFd;
    format!("/proc/{}/fd/{}", std::process::id(), file.as_raw_fd())
}

/// Whether `set-on` puts the picture on the desktop: `background`, or `both`, the lock screen
/// showing the desktop's picture under Glass; not the lock screen alone, which has none of its own.
fn sets_the_desktop(s: &str) -> bool {
    matches!(s, "background" | "both")
}

/// The local path of a `file://` URI. The frontend turns a file descriptor into the host path
/// it names, so a sandboxed app can name any file the user can read.
pub(crate) fn file_path(uri: &str) -> Option<PathBuf> {
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

/// Whether a remembered Allow covers the picture at `real`, the path its file resolved to: only
/// for a file in `folders`, which the app reads itself. An Allow given without the picture shown
/// would otherwise let the app put any picture the user owns on the desktop unseen.
fn remembered_applies(real: &Path, folders: &[PathBuf]) -> bool {
    folders.iter().any(|f| real.starts_with(f))
}

/// The folders a sandboxed `app_id` reads itself, resolved: the document portal's and its own
/// data folder. None for an empty app id.
fn app_folders(app_id: &str) -> Vec<PathBuf> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from);
    let home = std::env::var_os("HOME").map(PathBuf::from);
    app_folders_in(app_id, runtime.as_deref(), home.as_deref())
}

fn app_folders_in(app_id: &str, runtime: Option<&Path>, home: Option<&Path>) -> Vec<PathBuf> {
    use std::path::Component;
    let mut parts = Path::new(app_id).components();
    if !matches!(
        (parts.next(), parts.next()),
        (Some(Component::Normal(_)), None)
    ) {
        return Vec::new();
    }
    let doc = runtime.map(|r| r.join("doc"));
    let own = home.map(|h| h.join(".var/app").join(app_id));
    [doc, own]
        .into_iter()
        .flatten()
        .filter_map(|f| std::fs::canonicalize(f).ok())
        .collect()
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| crate::dialog::one_line(&n.to_string_lossy()))
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| tr!("wallpaper-default-name"))
}

/// The regular file at `path` in a sealed memfd when its content is of a format the desktop shows
/// ([`reprisefs::pictures::Format`]), no larger than [`reprisekit::wallpaper::MAX_BYTES`] and no
/// side past [`reprisekit::wallpaper::MAX_SIDE`], and the path the opened file resolved to.
pub fn read_picture(path: &Path) -> Option<(File, PathBuf)> {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::OpenOptionsExt;
    let f = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOCTTY)
        .open(path)
        .ok()?;
    let meta = f.metadata().ok()?;
    let cap = reprisekit::wallpaper::MAX_BYTES;
    if !meta.file_type().is_file() || meta.len() > cap {
        return None;
    }
    let real = std::fs::read_link(format!("/proc/self/fd/{}", f.as_raw_fd())).ok()?;
    let picture = sealed_copy(f, cap).ok()?;
    let (w, h) = dimensions(&picture)?;
    let max = reprisekit::wallpaper::MAX_SIDE;
    (w <= max && h <= max).then_some((picture, real))
}

/// The size `picture`'s header states, read from its start; None for a format the desktop does not
/// show. JPEG XL and AVIF state it in their first [`reprisefs::imagesize::HEAD_BYTES`].
fn dimensions(picture: &File) -> Option<(u32, u32)> {
    use reprisefs::pictures::Format;
    use std::io::{Read, Seek};
    let mut reader = std::io::BufReader::new(picture);
    reader.rewind().ok()?;
    let mut head = Vec::new();
    (&mut reader)
        .take(reprisefs::imagesize::HEAD_BYTES as u64)
        .read_to_end(&mut head)
        .ok()?;
    let mut header = |format| {
        reader.rewind().ok()?;
        image::ImageReader::with_format(&mut reader, format)
            .into_dimensions()
            .ok()
    };
    match Format::of(&head)? {
        Format::Jxl | Format::Avif => reprisefs::imagesize::dimensions(&head),
        Format::Bmp => header(image::ImageFormat::Bmp),
        Format::Jpeg => header(image::ImageFormat::Jpeg),
        Format::Png => header(image::ImageFormat::Png),
    }
}

/// Copy `picture`, from its start, as `name` into `dir`, point `[wallpaper]` in `config` at the
/// copy and delete the copy in `dir` the config named before.
fn install(dir: &Path, name: &str, mut picture: &File, config: &Path) -> Result<PathBuf, String> {
    use std::io::Seek;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let (mut file, to) =
        reprisefs::ops::create_new_numbered(dir, name).map_err(|e| e.to_string())?;
    let mut before = None;
    let written = picture
        .rewind()
        .and_then(|()| std::io::copy(&mut picture, &mut file))
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())
        .and_then(|()| {
            repriseconf::edit(config, |text| {
                let mut doc: DocumentMut = text.parse().map_err(|e| format!("{e}"))?;
                before = repriseconf::get(&doc, &IMAGE)
                    .and_then(|i| i.as_str())
                    .map(PathBuf::from);
                apply(&mut doc, &to)?;
                Ok::<_, String>(doc.to_string())
            })
            .map_err(|e| e.to_string())?
            .map(drop)
        });
    if let Err(e) = written {
        let _ = std::fs::remove_file(&to);
        return Err(e);
    }
    if let Some(old) = before.filter(|old| old.parent() == Some(dir) && *old != to) {
        let _ = std::fs::remove_file(old);
    }
    Ok(to)
}

/// Set `image` as the picture, ending a slideshow, which would show in its place.
fn apply(doc: &mut DocumentMut, image: &Path) -> Result<(), String> {
    let image = image.to_str().ok_or("the picture's path is not UTF-8")?;
    repriseconf::set(doc, &IMAGE, Value::from(image)).map_err(|e| e.to_string())?;
    repriseconf::remove(doc, &SLIDESHOW);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_preview_reads_the_bytes_it_was_given_and_cannot_be_rewritten() {
        use std::io::Write;
        let mut file = sealed_copy(&b"picture"[..], 64).unwrap();
        let path = fd_path(&file);
        assert_eq!(std::fs::read(&path).unwrap(), b"picture");
        assert!(file.write_all(b"other").is_err(), "the memfd is sealed");
        let reopened = std::fs::OpenOptions::new().write(true).open(&path);
        assert!(reopened.and_then(|mut f| f.write_all(b"other")).is_err());
        drop(file);
        assert!(
            std::fs::read(&path).is_err(),
            "the preview goes with its file"
        );
    }

    #[test]
    fn the_prompt_previews_the_copy_by_its_path() {
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(4, 3, image::Rgba([9, 9, 9, 255]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let copy = sealed_copy(png.as_slice(), reprisekit::wallpaper::MAX_BYTES).unwrap();
        let file = File::open(fd_path(&copy)).unwrap();
        let shown = crate::wallpaperpreview::preview(file, (64, 64)).unwrap();
        assert_eq!((shown.width, shown.height), (4, 3));
    }

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
        assert!(sets_the_desktop("background"));
        assert!(sets_the_desktop("both"));
        assert!(!sets_the_desktop("lockscreen"));
        assert!(!sets_the_desktop("elsewhere"));
    }

    #[test]
    fn the_picture_replaces_a_slideshow() {
        let mut doc: DocumentMut = "# mine\n[wallpaper]\nslideshow = \"~/Pictures\"\n\
                                    position = \"center\"\n"
            .parse()
            .unwrap();
        let pic = Path::new("/home/u/.local/share/reprise/wallpapers/dunes.jpg");
        apply(&mut doc, pic).unwrap();
        let cfg = repriseconf::config::Config::parse(&doc.to_string()).unwrap();
        assert_eq!(cfg.wallpaper.image.as_deref(), Some(pic));
        assert_eq!(cfg.wallpaper.slideshow, None);
        assert!(doc.to_string().starts_with("# mine\n"));
        assert!(doc.to_string().contains("position = \"center\""));
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
        let (copy, real) = read_picture(&png).unwrap();
        assert_eq!(real, std::fs::canonicalize(&png).unwrap());
        assert_eq!(std::fs::read(fd_path(&copy)).unwrap(), bytes);
        let text = dir.path().join("note.png");
        std::fs::write(&text, "not a picture").unwrap();
        assert!(read_picture(&text).is_none());
        assert!(read_picture(dir.path()).is_none());
        let jxl = dir.path().join("red.jxl");
        std::fs::write(&jxl, crate::access::RED_2X2_JXL).unwrap();
        assert!(read_picture(&jxl).is_some(), "a JPEG XL by its size header");
        let cut = dir.path().join("cut.jxl");
        std::fs::write(&cut, [0xff, 0x0a]).unwrap();
        assert!(read_picture(&cut).is_none(), "a JPEG XL stating no size");
        for format in [
            image::ImageFormat::Gif,
            image::ImageFormat::WebP,
            image::ImageFormat::Tiff,
        ] {
            let mut other = Vec::new();
            image::RgbaImage::from_pixel(2, 2, image::Rgba([9, 9, 9, 255]))
                .write_to(&mut std::io::Cursor::new(&mut other), format)
                .unwrap();
            let path = dir.path().join("other");
            std::fs::write(&path, &other).unwrap();
            assert!(
                read_picture(&path).is_none(),
                "the desktop shows no {format:?}"
            );
        }
        let to = dir.path().join("wallpapers");
        assert_eq!(file_name(&png), "dot.png");
        let config = dir.path().join("config.toml");
        let copied = install(&to, "dot.png", &copy, &config).unwrap();
        assert_eq!(copied, to.join("dot.png"));
        assert_eq!(std::fs::read(&copied).unwrap(), bytes);
        let written = std::fs::read_to_string(&config).unwrap();
        assert!(
            written.contains(&format!("image = \"{}\"", copied.display())),
            "{written}"
        );
    }

    fn memfd(bytes: &[u8]) -> File {
        sealed_copy(bytes, reprisekit::wallpaper::MAX_BYTES).unwrap()
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
        assert!(read_picture(&wide).is_none());
        let widest = dir.path().join("widest.png");
        std::fs::write(&widest, png(max, 1)).unwrap();
        assert!(read_picture(&widest).is_some());
    }

    #[test]
    fn a_new_wallpaper_deletes_the_copies_before_it() {
        let dir = tempfile::tempdir().unwrap();
        let to = dir.path().join("wallpapers");
        let config = dir.path().join("config.toml");
        let picture = memfd(&png(1, 1));
        let first = install(&to, "a.png", &picture, &config).unwrap();
        let again = install(&to, "a.png", &picture, &config).unwrap();
        assert_ne!(first, again);
        let last = install(&to, "b.png", &picture, &config).unwrap();
        let left: Vec<PathBuf> = std::fs::read_dir(&to)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        assert_eq!(left, [last]);
    }

    #[test]
    fn a_file_in_the_folder_the_config_never_named_survives() {
        let dir = tempfile::tempdir().unwrap();
        let to = dir.path().join("wallpapers");
        let config = dir.path().join("config.toml");
        std::fs::create_dir_all(&to).unwrap();
        let own = to.join("my-own-photo.jpg");
        std::fs::write(&own, b"photo").unwrap();
        let picture = memfd(&png(1, 1));
        let first = install(&to, "a.png", &picture, &config).unwrap();
        let last = install(&to, "b.png", &picture, &config).unwrap();
        assert!(own.exists(), "my-own-photo.jpg is kept");
        assert!(!first.exists(), "a.png is replaced by b.png");
        assert!(last.exists());
    }

    #[test]
    fn the_method_is_named_set_wallpaper_uri_in_capitals() {
        use zbus::object_server::Interface;
        let mut xml = String::new();
        let portal = Wallpaper {
            frontend: crate::Frontend::default(),
            read: read_picture,
            open: Open::default(),
        };
        portal.introspect_to_writer(&mut xml, 0);
        assert!(xml.contains(r#"<method name="SetWallpaperURI">"#), "{xml}");
    }

    #[test]
    fn a_remembered_allow_covers_only_a_picture_the_app_reads_itself() {
        let dir = tempfile::tempdir().unwrap();
        let dir = std::fs::canonicalize(dir.path()).unwrap();
        let (run, home) = (dir.join("run"), dir.join("home"));
        let doc = run.join("doc/3f2a91c0");
        let own = home.join(".var/app/org.example.Photos");
        let pictures = home.join("Pictures");
        for (folder, name) in [
            (&doc, "sunrise.png"),
            (&own, "cache.png"),
            (&pictures, "id.png"),
        ] {
            std::fs::create_dir_all(folder).unwrap();
            std::fs::write(folder.join(name), png(1, 1)).unwrap();
        }
        std::os::unix::fs::symlink(pictures.join("id.png"), own.join("link.png")).unwrap();
        let folders = app_folders_in("org.example.Photos", Some(&run), Some(&home));
        let applies = |p: PathBuf| remembered_applies(&read_picture(&p).unwrap().1, &folders);
        assert!(
            applies(doc.join("sunrise.png")),
            "a document portal file is silent"
        );
        assert!(
            applies(own.join("cache.png")),
            "a file in its own folder is silent"
        );
        assert!(!applies(pictures.join("id.png")), "a file outside asks");
        assert!(
            !applies(own.join("link.png")),
            "a link out of its folder asks"
        );
        assert!(
            !applies(own.join("../../../Pictures/id.png")),
            "a path out asks"
        );
        let other = app_folders_in("org.example.Other", Some(&run), Some(&home));
        assert!(!remembered_applies(&own.join("cache.png"), &other));
        for id in ["", "..", ".", "a/b", "/home"] {
            assert!(
                app_folders_in(id, Some(&run), Some(&home)).is_empty(),
                "{id:?}"
            );
        }
    }

    #[test]
    fn a_sandboxed_apps_picture_is_always_shown_in_the_prompt() {
        assert!(shows_preview("org.example.Photos", false));
        assert!(shows_preview("", true));
        assert!(!shows_preview("", false));
    }

    static READ: crate::tests::Gate = crate::tests::Gate::new();

    static HELD: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    static HELD_OF_A: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    /// A memfd for a fake read, its inode noted in `held`.
    fn held_memfd(held: &std::sync::atomic::AtomicU64) -> File {
        use std::os::unix::fs::MetadataExt;
        let file = sealed_copy(&b"picture"[..], 64).unwrap();
        let ino = file.metadata().unwrap().ino();
        held.store(ino, std::sync::atomic::Ordering::SeqCst);
        file
    }

    /// Whether this process still holds the memfd [`held_memfd`] noted in `held`.
    fn holds_the_memfd(held: &std::sync::atomic::AtomicU64) -> bool {
        use std::os::unix::fs::MetadataExt;
        let ino = held.load(std::sync::atomic::Ordering::SeqCst);
        std::fs::read_dir("/proc/self/fd")
            .unwrap()
            .flatten()
            .any(|fd| {
                let memfd = std::fs::read_link(fd.path())
                    .is_ok_and(|l| l.to_string_lossy().starts_with("/memfd:reprise-wallpaper"));
                memfd && std::fs::metadata(fd.path()).is_ok_and(|m| m.ino() == ino)
            })
    }

    fn read_held_open(_: &Path) -> Option<(File, PathBuf)> {
        READ.wait();
        Some((held_memfd(&HELD), PathBuf::from("/tmp/a.png")))
    }

    #[test]
    fn a_close_while_the_picture_is_read_answers_cancelled_and_leaves_no_request() {
        let Some(bus) = crate::lockmode::tests::bus() else {
            eprintln!("no dbus-daemon; skipped");
            return;
        };
        zbus::block_on(async {
            let (client, portal) = crate::tests::portal_with(&bus, |frontend| Wallpaper {
                frontend,
                read: read_held_open,
                open: Open::default(),
            })
            .await;
            let handle = format!("{}1_1/w", crate::REQUEST_PREFIX);
            let set = zruntime::spawn({
                let (client, portal, handle) = (client.clone(), portal.clone(), handle.clone());
                async move {
                    let none: HashMap<&str, zbus::Variant<'_>> = HashMap::new();
                    let handle = ObjectPath::try_from(handle.as_str()).unwrap();
                    let body = (handle, "org.example.App", "", "file:///tmp/a.png", none);
                    let reply = crate::tests::call_portal(
                        &client,
                        &portal,
                        "Wallpaper",
                        "SetWallpaperURI",
                        &body,
                    );
                    reply.await.unwrap().body().deserialize::<u32>().unwrap()
                }
            });
            assert!(READ.reached().await, "the call reads the picture");
            let closed = crate::tests::close_request(&client, &portal, &handle).await;
            assert!(closed.is_ok(), "the Close is answered: {closed:?}");
            READ.open();
            assert_eq!(set.await.unwrap(), crate::RESPONSE_CANCELLED);
            assert!(
                !crate::tests::request_served(&portal, &handle).await,
                "no Request object stays behind"
            );
            assert!(!holds_the_memfd(&HELD), "the picture's memfd is closed");
        });
    }

    static FIRST_READ: crate::tests::Gate = crate::tests::Gate::new();
    static READS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

    /// The first read waits on [`FIRST_READ`]; every later one finds no picture at once.
    fn first_read_held_open(_: &Path) -> Option<(File, PathBuf)> {
        if READS.fetch_add(1, std::sync::atomic::Ordering::SeqCst) > 0 {
            return None;
        }
        FIRST_READ.wait();
        Some((
            sealed_copy(std::io::empty(), 0).unwrap(),
            PathBuf::from("/tmp/a.png"),
        ))
    }

    async fn set_wallpaper_of(
        client: &zbus::Connection,
        portal: &zbus::Connection,
        handle: &str,
        app_id: &str,
        uri: &str,
    ) -> zbus::Result<zbus::Message> {
        let none: HashMap<&str, zbus::Variant<'_>> = HashMap::new();
        let handle = ObjectPath::try_from(handle).unwrap();
        let body = (handle, app_id, "", uri, none);
        crate::tests::call_portal(client, portal, "Wallpaper", "SetWallpaperURI", &body).await
    }

    async fn set_wallpaper(
        client: &zbus::Connection,
        portal: &zbus::Connection,
        handle: &str,
    ) -> zbus::Result<zbus::Message> {
        set_wallpaper_of(
            client,
            portal,
            handle,
            "org.example.App",
            "file:///tmp/a.png",
        )
        .await
    }

    #[test]
    fn a_second_call_on_a_served_handle_fails_and_the_first_keeps_its_request() {
        let Some(bus) = crate::lockmode::tests::bus() else {
            eprintln!("no dbus-daemon; skipped");
            return;
        };
        zbus::block_on(async {
            let (client, portal) = crate::tests::portal_with(&bus, |frontend| Wallpaper {
                frontend,
                read: first_read_held_open,
                open: Open::default(),
            })
            .await;
            let handle = format!("{}1_1/twice", crate::REQUEST_PREFIX);
            let first = zruntime::spawn({
                let (client, portal, handle) = (client.clone(), portal.clone(), handle.clone());
                async move {
                    let reply = set_wallpaper(&client, &portal, &handle).await.unwrap();
                    reply.body().deserialize::<u32>().unwrap()
                }
            });
            assert!(
                FIRST_READ.reached().await,
                "the first call reads the picture"
            );
            let second = set_wallpaper(&client, &portal, &handle).await;
            assert!(second.is_err(), "the second call fails: {second:?}");
            assert!(
                crate::tests::request_served(&portal, &handle).await,
                "the first call's Request is still served"
            );
            let closed = crate::tests::close_request(&client, &portal, &handle).await;
            assert!(closed.is_ok(), "the Close is answered: {closed:?}");
            FIRST_READ.open();
            assert_eq!(first.await.unwrap(), crate::RESPONSE_CANCELLED);
            assert!(!crate::tests::request_served(&portal, &handle).await);
        });
    }

    #[test]
    fn a_picture_is_read_once_into_a_sealed_memfd() {
        use std::os::fd::AsRawFd;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("grey.png");
        let bytes = png(300, 200);
        std::fs::write(&path, &bytes).unwrap();
        let (copy, _) = read_picture(&path).unwrap();
        let link = std::fs::read_link(fd_path(&copy)).unwrap();
        assert!(
            link.to_string_lossy()
                .starts_with("/memfd:reprise-wallpaper"),
            "{link:?}"
        );
        // SAFETY: `copy` is an open file; F_GET_SEALS takes no argument.
        let seals = unsafe { libc::fcntl(copy.as_raw_fd(), libc::F_GET_SEALS) };
        let all = libc::F_SEAL_WRITE | libc::F_SEAL_SHRINK | libc::F_SEAL_GROW | libc::F_SEAL_SEAL;
        assert_eq!(seals & all, all, "sealed before it is handed on");
        assert_eq!(std::fs::read(fd_path(&copy)).unwrap(), bytes);
        let to = dir.path().join("wallpapers");
        let config = dir.path().join("config.toml");
        let installed = install(&to, "grey.png", &copy, &config).unwrap();
        assert_eq!(std::fs::read(installed).unwrap(), bytes);
    }

    #[test]
    fn a_copy_stops_reading_past_its_cap() {
        use std::io::Read;
        let cap = 1 << 20;
        let endless = sealed_copy(std::io::repeat(7), cap);
        assert_eq!(
            endless.map(drop).map_err(|e| e.kind()),
            Err(std::io::ErrorKind::FileTooLarge)
        );
        let full = sealed_copy(std::io::repeat(7).take(cap), cap).unwrap();
        assert_eq!(full.metadata().unwrap().len(), cap);
    }

    #[test]
    fn a_file_past_the_desktops_budget_is_refused_unread() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("huge.png");
        std::fs::write(&path, png(1, 1)).unwrap();
        let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        file.set_len(reprisekit::wallpaper::MAX_BYTES + 1).unwrap();
        assert!(read_picture(&path).is_none());
    }

    #[test]
    fn an_app_has_one_request_open_and_all_apps_at_most_four() {
        let open = Open::default();
        let first = open.take("org.example.A").unwrap();
        assert!(
            open.take("org.example.A").is_none(),
            "a second of A's waits for none"
        );
        drop(first);
        let again = open.take("org.example.A");
        assert!(again.is_some(), "A asks again once its first request ends");
        let others: Vec<Turn> = ["B", "C", "D"]
            .iter()
            .map(|id| open.take(&format!("org.example.{id}")).unwrap())
            .collect();
        assert_eq!(others.len() + 1, MAX_OPEN);
        assert!(open.take("org.example.E").is_none(), "past MAX_OPEN");
        drop(again);
        assert!(open.take("org.example.E").is_some());
    }

    static HELD_READ: crate::tests::Gate = crate::tests::Gate::new();
    static OTHER_READS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

    /// `/tmp/held.png` waits on [`HELD_READ`]; any other path counts in [`OTHER_READS`] and is no
    /// picture.
    fn held_or_counted(path: &Path) -> Option<(File, PathBuf)> {
        if path != Path::new("/tmp/held.png") {
            OTHER_READS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            return None;
        }
        HELD_READ.wait();
        Some((held_memfd(&HELD_OF_A), path.to_owned()))
    }

    #[test]
    fn a_second_request_of_an_app_with_one_open_is_refused_unread() {
        use std::sync::atomic::Ordering::SeqCst;
        let Some(bus) = crate::lockmode::tests::bus() else {
            eprintln!("no dbus-daemon; skipped");
            return;
        };
        zbus::block_on(async {
            let (client, portal) = crate::tests::portal_with(&bus, |frontend| Wallpaper {
                frontend,
                read: held_or_counted,
                open: Open::default(),
            })
            .await;
            let handle = |n: u32| format!("{}1_1/app{n}", crate::REQUEST_PREFIX);
            let answer = |reply: zbus::Result<zbus::Message>| {
                reply.unwrap().body().deserialize::<u32>().unwrap()
            };
            let (a, b, other) = ("org.example.A", "org.example.B", "file:///tmp/other.png");
            let first = zruntime::spawn({
                let (client, portal, handle) = (client.clone(), portal.clone(), handle(1));
                async move {
                    answer(
                        set_wallpaper_of(&client, &portal, &handle, a, "file:///tmp/held.png")
                            .await,
                    )
                }
            });
            assert!(
                HELD_READ.reached().await,
                "A's first call reads its picture"
            );
            let second = set_wallpaper_of(&client, &portal, &handle(2), a, other).await;
            assert_eq!(answer(second), crate::RESPONSE_OTHER);
            assert_eq!(OTHER_READS.load(SeqCst), 0, "A's second call reads nothing");
            let of_b = set_wallpaper_of(&client, &portal, &handle(3), b, other).await;
            assert_eq!(answer(of_b), crate::RESPONSE_OTHER);
            assert_eq!(OTHER_READS.load(SeqCst), 1, "B's call is read meanwhile");
            let closed = crate::tests::close_request(&client, &portal, &handle(1)).await;
            assert!(closed.is_ok(), "the Close is answered: {closed:?}");
            HELD_READ.open();
            assert_eq!(first.await.unwrap(), crate::RESPONSE_CANCELLED);
            assert!(
                !holds_the_memfd(&HELD_OF_A),
                "the first call's memfd is closed"
            );
            let again = set_wallpaper_of(&client, &portal, &handle(4), a, other).await;
            assert_eq!(answer(again), crate::RESPONSE_OTHER);
            assert_eq!(OTHER_READS.load(SeqCst), 2, "A's next call is read");
        });
    }
}
