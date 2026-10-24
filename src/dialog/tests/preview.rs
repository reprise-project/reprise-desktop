use super::*;

/// Renders Browse to `target/browse-preview.png` for a look. Run with
/// `cargo test render_browse_preview -- --ignored`; `REPRISE_PREVIEW_DIR`
/// names the folder it lists.
#[test]
#[ignore]
fn render_browse_preview() {
    use crate::icontheme::{IconCache, IconTheme};
    let theme = Theme::default();
    let text = TextRenderer::load(&FontConfig::default());
    let name = std::env::var("REPRISE_PREVIEW_ICONS").unwrap_or_else(|_| "breeze".into());
    let mut icons = IconCache::new(IconTheme::load(&name, &IconTheme::default_roots()));
    let dir = std::env::var("REPRISE_PREVIEW_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());
    let mut picker = crate::filepick::FilePicker::run_command(
        &dir,
        crate::filepick::icons(&mut icons, 1, reprisekit::theme::TextUnits::ONE),
        true,
    );
    for r in picker.take_requests() {
        picker.reply(r.run(), &mut icons, 1);
    }
    let d = Dialog::browse(picker);
    let l = DialogLayout::compute(&theme, &text, &d);
    let c = draw(
        &theme,
        &text,
        &l,
        None,
        None,
        &[],
        &[],
        None,
        &d,
        false,
        None,
        Cues::NONE,
        true,
        None,
    );
    reprisekit::raster::write_preview(&c, "browse-preview.png");
}

/// Renders the key aid prompts to `target/key-aid-preview.png`. Run
/// with `cargo test key_aid_preview -- --ignored`.
#[test]
#[ignore]
fn render_key_aid_preview() {
    use crate::keyfilter::Aid;
    let theme = Theme::default();
    let text = TextRenderer::load(&FontConfig::default());
    let canvases: Vec<Canvas> = [Aid::StickyKeys, Aid::SlowKeys, Aid::MouseKeys]
        .into_iter()
        .map(|aid| {
            let d = Dialog::key_aid_prompt(aid);
            let l = DialogLayout::compute(&theme, &text, &d);
            draw(
                &theme,
                &text,
                &l,
                None,
                None,
                &[],
                &[],
                None,
                &d,
                true,
                None,
                Cues::ALL,
                true,
                None,
            )
        })
        .collect();
    let mut out = Canvas::new(410, 2 * 163 + 8);
    out.fill_rect(out.bounds(), theme.colors.background);
    out.blit_canvas(0, 0, &canvases[0]);
    out.blit_canvas(0, 171, &canvases[1]);
    reprisekit::raster::write_preview(&out, "key-aid-preview.png");
}

/// Renders the dialogs to `target/dialogs-preview.png`, and under Blue
/// to `target/dialogs-blue-preview.png`, for eyeballing. Run with
/// `cargo test preview -- --ignored`.
#[test]
#[ignore]
fn render_dialogs_preview() {
    for (theme, name) in [
        (Theme::default(), "dialogs-preview.png"),
        (blue(), "dialogs-blue-preview.png"),
    ] {
        dialogs_preview(&theme, name);
    }
}

fn dialogs_preview(theme: &Theme, file: &str) {
    use crate::icontheme::{IconCache, IconTheme};
    let theme = theme.clone();
    let text = TextRenderer::load(&FontConfig::default());
    let name = std::env::var("REPRISE_PREVIEW_ICONS").unwrap_or_else(|_| "breeze".into());
    let mut icons = IconCache::new(IconTheme::load(&name, &IconTheme::default_roots()));
    let run = Dialog::run(&history());
    let mut open = Dialog::run(&history());
    open.key(Key::Drop);
    let props = Dialog::properties(&file_info(), "text-x-generic", (0, 0));
    let mut folder = file_info();
    folder.name = "New Folder".into();
    folder.hidden_ext = String::new();
    folder.path = "/home/u/Desktop/New Folder".into();
    folder.is_dir = true;
    folder.opens_with = None;
    folder.type_name = "File Folder".into();
    folder.contains = Some((2, 1));
    let folder = Dialog::properties(&folder, "folder", (0, 0));
    let dialogs = [
        run,
        open,
        props,
        folder,
        chooser(),
        {
            let mut c = chooser();
            c.key(Key::Up);
            c
        },
        attr_confirm(),
        Dialog::advanced_attributes(
            Subject::File,
            None,
            &text,
            theme.fonts.size_px,
            TextUnits::ONE,
        ),
        Dialog::advanced_attributes(
            Subject::Folder,
            None,
            &text,
            theme.fonts.size_px,
            TextUnits::ONE,
        ),
        Dialog::confirm_hide(crate::desktop::Place::Computer, "Computer"),
        Dialog::end_program("signal-desktop", 1234),
        Dialog::log_off(),
        Dialog::turn_off(&power()),
    ];
    let canvases: Vec<Canvas> = dialogs
        .into_iter()
        .map(|mut d| {
            d.fit(&theme, &text);
            let d = &d;
            let l = DialogLayout::compute(&theme, &text, d);
            let icon = d.icon.as_deref().and_then(|n| icons.get(n, ICON_PX));
            let caption = d
                .icon
                .as_deref()
                .and_then(|n| icons.get(n, theme.icon_size()));
            let button_icons: Vec<Option<Canvas>> = d
                .buttons
                .iter()
                .map(|b| b.icon.as_deref().and_then(|n| icons.get(n, ICON_PX)))
                .collect();
            let logo = icons.get("start-here", ICON_PX);
            draw(
                &theme,
                &text,
                &l,
                icon.as_ref(),
                caption.as_ref(),
                &button_icons,
                &[],
                logo.as_ref(),
                d,
                true,
                None,
                Cues::ALL,
                true,
                None,
            )
        })
        .collect();
    let gap = 8;
    let w = canvases.iter().map(|c| c.width).max().unwrap() as i32 + 2 * gap;
    let h: i32 = canvases.iter().map(|c| c.height as i32 + gap).sum::<i32>() + gap;
    let mut out = Canvas::new(w as u32, h as u32);
    out.fill_rect(out.bounds(), theme.colors.background);
    let mut y = gap;
    for c in &canvases {
        out.blit_canvas(gap, y, c);
        y += c.height as i32 + gap;
    }
    reprisekit::raster::write_preview(&out, file);
}

/// Renders the tabbed sheets under each interface to
/// `target/sheets-{classic,blue,glass}.png`: file and set Properties,
/// Recycle Bin Properties on both tabs with Apply lit and the pointer
/// over the closed tab. Run with
/// `cargo test render_sheets_preview -- --ignored`.
#[test]
#[ignore]
fn render_sheets_preview() {
    use crate::binprops::{self, BinSheet, Drive};
    use reprisekit::interface::Interface;
    for (i, file) in [
        (Interface::Classic, "sheets-classic.png"),
        (Interface::Blue, "sheets-blue.png"),
        (Interface::Glass, "sheets-glass.png"),
    ] {
        let theme = reprisekit::interfaces::theme_for(i);
        let text = TextRenderer::load(&FontConfig::default());
        let px = theme.fonts.size_px;
        let styled = theme.look.lights_pointer();
        let bin = |tab: usize| {
            let mut s = BinSheet::new(
                reprisefs::bincap::Policy::default(),
                vec![Drive {
                    key: "home".into(),
                    label: "Local Disk (/home)".into(),
                    size: 21_365_735_424,
                }],
            );
            s.pending.global.remove_immediately = true;
            let mut d = binprops::dialog(s, (0, 0), "user-trash-full");
            if tab == 1 {
                d.cycle_page(false);
            }
            d
        };
        let dialogs = [
            Dialog::properties(&file_info(), "text-x-generic", (0, 0)),
            Dialog::set_properties(&a_set(), "Reports", (0, 0)),
            bin(0),
            bin(1),
        ];
        let canvases: Vec<Canvas> = dialogs
            .into_iter()
            .map(|mut d| {
                d.fit(&theme, &text);
                let l = DialogLayout::compute(&theme, &text, &d);
                if let Some(t) = l.tabs.last().filter(|_| l.tabs.len() > 1) {
                    let (x, y) = (t.x + 4, t.y + 6);
                    let over = l.hit(&d, x, y, &text, px);
                    d.hover(over, (x, y), styled);
                }
                draw(
                    &theme,
                    &text,
                    &l,
                    None,
                    None,
                    &[],
                    &[],
                    None,
                    &d,
                    false,
                    None,
                    Cues::ALL,
                    true,
                    None,
                )
            })
            .collect();
        let gap = 8;
        let w = canvases.iter().map(|c| c.width).max().unwrap() as i32 + 2 * gap;
        let h: i32 = canvases.iter().map(|c| c.height as i32 + gap).sum::<i32>() + gap;
        let mut out = Canvas::new(w as u32, h as u32);
        out.fill_rect(out.bounds(), theme.colors.background);
        let mut y = gap;
        for c in &canvases {
            out.blit_canvas(gap, y, c);
            y += c.height as i32 + gap;
        }
        reprisekit::raster::write_preview(&out, file);
    }
}
