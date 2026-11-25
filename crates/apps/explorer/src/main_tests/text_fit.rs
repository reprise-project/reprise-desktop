use super::*;
use reprisekit::interface::Interface;

/// A folder of a few files and a subfolder in the test session's HOME, each modified 2026-01-02
/// and none with an extension the machine's MIME database names, so the view's text is the same on
/// every machine, and the Folders tree reaches it through Home without listing the temp folder.
fn sample_folder() -> tempfile::TempDir {
    let home = std::env::var_os("HOME").expect("the test session's HOME");
    let dir = tempfile::Builder::new()
        .prefix("Reports")
        .rand_bytes(0)
        .tempdir_in(home)
        .unwrap();
    std::fs::create_dir(dir.path().join("Quarterly Figures")).unwrap();
    let when = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_767_355_200);
    for name in ["Budget 2026", "notes", "Team photo"] {
        let f = std::fs::File::create(dir.path().join(name)).unwrap();
        f.set_len(48_213).unwrap();
        f.set_modified(when).unwrap();
    }
    dir
}

/// The window on a sample folder in each view mode and interface, at scale 1 and 2, drawn with
/// Tahoma and as on a machine without it, lists each line DejaVu Sans cuts or ellipsizes more
/// often than Tahoma against `tests/pins/text_fit_without_tahoma.txt`.
#[test]
fn the_window_without_tahoma_fits_the_text_it_fits_with_tahoma() {
    let sample = sample_folder();
    let dir = sample.path().to_path_buf();
    let found = reprisekit::pins::unfit_without_tahoma(|| {
        for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
            let mut ctx = test_ctx();
            ctx.theme = reprisekit::interfaces::theme_for(i);
            ctx.text = TextRenderer::load(&ctx.theme.fonts).into();
            let mut ex = test_explorer(Place::Folder(dir.clone()), &mut ctx);
            for mode in [shellview::Mode::Details, shellview::Mode::Icons] {
                ex.sv.mode = mode;
                for s in [1, 2] {
                    let mut c = Canvas::with_scale(792, 573, s);
                    ctx.at_scale(s, |ctx| window::draw(&ex, &mut c, ctx));
                    reprisekit::pins::surface_drawn(&format!("{}@{s} {mode:?}", i.as_str()));
                }
            }
        }
    });
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/pins/text_fit_without_tahoma.txt");
    reprisekit::pins::check_list(&path, &found, "lines DejaVu Sans cuts and Tahoma does not");
}
