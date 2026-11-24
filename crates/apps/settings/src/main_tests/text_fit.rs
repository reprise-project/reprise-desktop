use super::*;

/// Two Startup entries and no keyboard layout registry, whatever the machine has installed.
fn sample_startup(app: &mut SettingsApp) {
    app.xkb.clear();
    app.layouts_changed(None);
    let entry = |id: &str, name: &str, exec: &str| reprisefs::autostart::Entry {
        id: id.into(),
        name: name.into(),
        exec: exec.into(),
        exec_line: exec.into(),
        hidden: false,
        only_show_in: Vec::new(),
        not_show_in: Vec::new(),
        try_exec: None,
        path: None,
        terminal: false,
        under_systemd: false,
        user: true,
        stand_in: false,
    };
    app.entries = vec![
        entry("notes.desktop", "Notes", "reprise-notes"),
        entry("sync.desktop", "File Sync", "/usr/bin/sync-daemon"),
    ];
    app.applied_checks = vec![true; 2];
    app.show_startup(&[]);
}

/// Every page in each interface at its smallest and at 1600x1000, at scale 1 and 2, drawn with
/// Tahoma and as on a machine without it, lists each line DejaVu Sans cuts or ellipsizes more
/// often than Tahoma against `tests/pins/text_fit_without_tahoma.txt`, by sheet: the page list
/// draws on every page.
#[test]
fn every_page_without_tahoma_fits_the_text_it_fits_with_tahoma() {
    let found = reprisekit::pins::unfit_without_tahoma(|| {
        for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
            for size in [None, Some((1600, 1000))] {
                let mut app = app(&format!("text-fit-{}", i.as_str()));
                sample_startup(&mut app);
                let mut ctx = test_ctx();
                ctx.theme = reprisekit::interfaces::theme_for(i);
                ctx.text = TextRenderer::load(&ctx.theme.fonts).into();
                if let Some(size) = size {
                    ctx.size = size;
                } else {
                    app.follow_text(&ctx);
                    ctx.size = app.min_window();
                }
                app.event(MAIN, Event::Resized, &mut ctx);
                let (w, h) = app.sheet.size;
                let at = size.map_or("smallest".into(), |(w, h)| format!("{w}x{h}"));
                for s in [1, 2] {
                    for p in Page::all() {
                        app.open_page(p, None);
                        let mut c = Canvas::with_scale(w as u32, h as u32, s);
                        ctx.at_scale(s, |ctx| app.paint(MAIN, &mut c, ctx));
                    }
                    reprisekit::pins::surface_drawn(&format!("{}@{s} {at}", i.as_str()));
                }
            }
        }
    });
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/pins/text_fit_without_tahoma.txt");
    reprisekit::pins::check_list(&path, &found, "lines DejaVu Sans cuts and Tahoma does not");
}
