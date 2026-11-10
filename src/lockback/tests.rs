use std::os::unix::net::UnixStream;

use super::*;
use crate::harness::Harness;
use crate::lockview;

/// A scratch folder holding `toml` as login.toml and a 4x4 red PNG as the background picture.
fn scratch(name: &str, toml: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("reprise-lockback-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("login.toml"), toml).unwrap();
    image::RgbImage::from_pixel(4, 4, image::Rgb([255, 0, 0]))
        .save_with_format(dir.join("background"), image::ImageFormat::Png)
        .unwrap();
    dir
}

/// Locked under `interface` with login.toml and the picture read from `dir`, under reprise-login
/// when `linked` (the daemon's end of the link returned), once the background read has landed.
fn locked(interface: &str, dir: &Path, linked: bool) -> (Harness, Option<UnixStream>) {
    let config = format!("[appearance]\ninterface = \"{interface}\"\n");
    let mut h = Harness::with_config(crate::config::Config::parse(&config).unwrap());
    h.output("TEST-1", 64, 48);
    h.state.lock.helpers.finger = |_| None;
    h.state.lock.back.config = dir.join("login.toml");
    h.state.lock.back.picture = dir.join("background");
    let daemon = linked.then(|| {
        let (ours, theirs) = UnixStream::pair().unwrap();
        crate::loginlink::install(&mut h.state, crate::loginlink::LoginLink::from_stream(ours));
        theirs
    });
    lockview::lock(&mut h.state);
    let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while h.state.lock.back.reading {
        assert!(std::time::Instant::now() < end, "the background read lands");
        h.run_for(5);
    }
    (h, daemon)
}

fn over_picture(h: &Harness) -> bool {
    h.state
        .lock
        .console
        .as_ref()
        .unwrap()
        .view
        .model
        .over_picture
}

/// Each cover's wall, if it has one, and whether it holds pixels.
fn walls(h: &Harness) -> Vec<Option<bool>> {
    let l = h.state.lock.console.as_ref().unwrap();
    (l.view.covers.iter())
        .map(|c| c.wall.map(|w| h.state.scene.buffer_has_pixels(w)))
        .collect()
}

#[test]
fn the_lock_shows_the_login_background_login_toml_names() {
    let toml = format!("[login]\nbackground = \"{BACKGROUND_PICTURE}\"\n");
    let dir = scratch("named", &toml);
    for interface in ["classic", "blue", "glass"] {
        let (h, _daemon) = locked(interface, &dir, true);
        assert!(
            over_picture(&h),
            "{interface}: the screen lies over the picture"
        );
        assert_eq!(
            walls(&h),
            [Some(true)],
            "{interface}: the picture fills the cover"
        );
        let shown = h
            .state
            .lock
            .back
            .get((64, 48))
            .expect("composed at the output's size");
        assert_eq!(
            shown.get(10, 10),
            0xFFFF_0000,
            "{interface}: the named picture, stretched"
        );
    }
}

#[test]
fn without_a_named_background_or_reprise_login_the_lock_keeps_its_backdrop() {
    let unset = scratch("unset", "[login]\nshow_users = true\n");
    let toml = format!("[login]\nbackground = \"{BACKGROUND_PICTURE}\"\n");
    let named = scratch("elsewhere", &toml);
    for (dir, linked, why) in [
        (&unset, true, "unset"),
        (&named, false, "another display manager"),
    ] {
        for interface in ["classic", "blue"] {
            let (h, _daemon) = locked(interface, dir, linked);
            assert!(!over_picture(&h), "{interface}, {why}");
            assert_eq!(
                walls(&h),
                [None],
                "{interface}, {why}: the backdrop colour alone"
            );
        }
        let (h, _daemon) = locked("glass", dir, linked);
        assert!(!over_picture(&h), "glass, {why}");
        assert!(
            !h.state.lock.back.shows(),
            "glass, {why}: the user's own wallpaper"
        );
    }
}

#[test]
fn a_background_set_while_unlocked_shows_on_the_next_lock() {
    let dir = scratch("later", "[login]\n");
    let (mut h, _daemon) = locked("blue", &dir, true);
    assert!(!over_picture(&h));
    lockview::unlock(&mut h.state);
    let toml = format!("[login]\nbackground = \"{BACKGROUND_PICTURE}\"\n");
    std::fs::write(dir.join("login.toml"), toml).unwrap();
    lockview::lock(&mut h.state);
    let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while h.state.lock.back.reading {
        assert!(std::time::Instant::now() < end, "the background read lands");
        h.run_for(5);
    }
    assert!(over_picture(&h));
    assert_eq!(walls(&h), [Some(true)]);
}
