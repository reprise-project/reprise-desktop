use super::*;
use crate::testfs::{mkfifo, within};
use std::fs;
use std::os::unix::fs::symlink;

const MAX: u64 = 1024;

fn root_of(dir: &Path) -> Confined {
    let flags = libc::O_PATH | libc::O_DIRECTORY;
    Confined::Root(open_at(cwd(), &c_path(dir).unwrap(), flags).unwrap())
}

/// `dirs` held open as a snap's folders, each by its own path.
fn beneath(dirs: &[PathBuf]) -> Confined {
    let flags = libc::O_PATH | libc::O_DIRECTORY | libc::O_NOFOLLOW;
    let open = |d: &PathBuf| Some((d.clone(), open_at(cwd(), &c_path(d).ok()?, flags).ok()?));
    Confined::Beneath(dirs.iter().filter_map(open).collect())
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

#[test]
fn a_flatpak_path_opens_inside_its_root() {
    let root = tempfile::tempdir().unwrap();
    let icon = "/app/share/icons/hicolor/16x16/apps/org.example.App.png";
    write(&root.path().join(&icon[1..]), "inside");
    let files = root_of(root.path());
    assert_eq!(files.read(Path::new(icon), MAX).unwrap(), b"inside");
    let listed = files.list(Path::new("/app/share/icons/hicolor/16x16/apps"), 8);
    assert_eq!(
        listed,
        HashSet::from([OsString::from("org.example.App.png")])
    );
    assert!(
        files.read(Path::new("app/share/x.png"), MAX).is_err(),
        "a relative path"
    );
}

#[test]
fn a_link_or_dotdot_out_of_a_flatpak_root_stays_inside_it() {
    let host = tempfile::tempdir().unwrap();
    let secret = host.path().join("secret.png");
    write(&secret, "host");
    let root = tempfile::tempdir().unwrap();
    let files = root_of(root.path());
    fs::create_dir(root.path().join("app")).unwrap();
    symlink(&secret, root.path().join("link.png")).unwrap();
    let up = format!("/app/../../../..{}", secret.display());
    for path in ["/link.png", up.as_str()] {
        assert!(
            files.read(Path::new(path), MAX).is_err(),
            "{path} reads nothing while the root has no {}",
            secret.display()
        );
    }
    write(
        &root.path().join(secret.strip_prefix("/").unwrap()),
        "sandbox",
    );
    for path in ["/link.png", up.as_str()] {
        assert_eq!(
            files.read(Path::new(path), MAX).unwrap(),
            b"sandbox",
            "{path} reads the root's own file"
        );
    }
}

#[test]
fn a_magic_link_inside_a_root_is_refused() {
    let host = tempfile::tempdir().unwrap();
    let secret = host.path().join("secret.png");
    write(&secret, "host");
    let files = root_of(Path::new("/"));
    assert_eq!(files.read(&secret, MAX).unwrap(), b"host");
    let through = format!("/proc/self/root{}", secret.display());
    assert!(files.read(Path::new(&through), MAX).is_err());
}

#[test]
fn only_a_small_regular_file_is_read() {
    let root = tempfile::tempdir().unwrap();
    mkfifo(&root.path().join("fifo.png"));
    fs::write(root.path().join("big.png"), vec![0u8; MAX as usize + 1]).unwrap();
    fs::create_dir(root.path().join("dir.png")).unwrap();
    let files = root_of(root.path());
    let read = within(move || {
        ["/fifo.png", "/big.png", "/dir.png"].map(|p| files.read(Path::new(p), MAX).is_err())
    });
    assert_eq!(read, [true; 3]);
}

#[test]
fn a_fifo_is_never_opened_for_reading() {
    let root = tempfile::tempdir().unwrap();
    let fifo = root.path().join("fifo.png");
    mkfifo(&fifo);
    let opened = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    // A writer's open returns once a reader opens the FIFO; it is given time to start waiting.
    let writer = std::thread::spawn({
        let (fifo, opened) = (fifo.clone(), opened.clone());
        move || {
            let _w = fs::OpenOptions::new().write(true).open(fifo);
            opened.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    });
    let settle = || std::thread::sleep(std::time::Duration::from_millis(200));
    settle();
    assert!(
        root_of(root.path())
            .read(Path::new("/fifo.png"), MAX)
            .is_err()
    );
    assert!(crate::ops::read_regular(&fifo, MAX).is_err());
    settle();
    assert!(
        !opened.load(std::sync::atomic::Ordering::SeqCst),
        "the FIFO was opened"
    );
    let _r = fs::File::open(&fifo).unwrap();
    writer.join().unwrap();
}

#[test]
fn a_list_keeps_at_most_max_names() {
    let root = tempfile::tempdir().unwrap();
    for i in 0..10 {
        write(&root.path().join(format!("d/{i}.png")), "");
    }
    let files = root_of(root.path());
    assert_eq!(files.list(Path::new("/d"), 4).len(), 4);
    assert_eq!(files.list(Path::new("/d"), 64).len(), 10);
}

#[test]
fn a_parallel_snaps_paths_by_its_name_open_beneath_its_instance() {
    let top = tempfile::tempdir().unwrap();
    let (mounted, home, runtime) = (
        top.path().join("snap"),
        top.path().join("home"),
        top.path().join("run"),
    );
    write(&mounted.join("chat_beta/1/icon.png"), "instance");
    write(&mounted.join("chat/1/icon.png"), "other instance");
    write(&home.join("chat_beta/common/x.png"), "home");
    write(&runtime.join("snap.chat_beta/x.png"), "runtime");
    let files = Confined::snap_under("chat_beta", &mounted, Some(&home), Some(&runtime));
    let read = |p: PathBuf| String::from_utf8(files.read(&p, MAX).ok()?).ok();
    let reads =
        |p: PathBuf, want: &str| assert_eq!(read(p.clone()).as_deref(), Some(want), "{p:?}");
    reads(mounted.join("chat/1/icon.png"), "instance");
    reads(mounted.join("chat_beta/1/icon.png"), "instance");
    reads(home.join("chat/common/x.png"), "home");
    reads(runtime.join("snap.chat/x.png"), "runtime");
    let plain = Confined::snap_under("chat", &mounted, Some(&home), Some(&runtime));
    let read = |p: PathBuf| plain.read(&p, MAX).ok();
    assert_eq!(
        read(mounted.join("chat_beta/1/icon.png")),
        None,
        "an instance's own folder"
    );
}

#[test]
fn a_snap_path_opens_only_beneath_its_own_folders() {
    let top = tempfile::tempdir().unwrap();
    let (own, other) = (top.path().join("snap/app"), top.path().join("other"));
    write(&own.join("current/meta/gui/icon.png"), "own");
    write(&other.join("x.png"), "other");
    symlink(other.join("x.png"), own.join("out.png")).unwrap();
    symlink("current/meta/gui/icon.png", own.join("in.png")).unwrap();
    let files = beneath(std::slice::from_ref(&own));
    let read = |p: PathBuf| files.read(&p, MAX).ok();
    assert_eq!(
        read(own.join("current/meta/gui/icon.png")).as_deref(),
        Some(&b"own"[..])
    );
    assert_eq!(
        read(own.join("in.png")).as_deref(),
        Some(&b"own"[..]),
        "a link that stays beneath"
    );
    assert_eq!(read(other.join("x.png")), None, "another folder");
    assert_eq!(read(own.join("../../other/x.png")), None, "a .. out");
    assert_eq!(read(own.join("out.png")), None, "a link out");
    assert_eq!(read(top.path().join("snap/application/x.png")), None);
}

#[test]
fn a_snap_folder_that_is_a_link_is_not_opened() {
    let top = tempfile::tempdir().unwrap();
    let other = top.path().join("other");
    write(&other.join("x.png"), "other");
    symlink(&other, top.path().join("app")).unwrap();
    let files = beneath(&[top.path().join("app")]);
    assert!(files.read(&top.path().join("app/x.png"), MAX).is_err());
    let Confined::Beneath(dirs) = Confined::snap("../app") else {
        panic!("a snap's files open beneath folders");
    };
    assert!(
        dirs.is_empty(),
        "a snap name holding a slash names no folder"
    );
}
