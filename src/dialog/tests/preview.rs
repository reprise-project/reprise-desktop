use super::*;

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
            let d = Dialog::key_aid_prompt(aid, true);
            let l = DialogLayout::compute(&theme, &text, &d);
            draw(
                &theme,
                &text,
                &l,
                None,
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
    let dialogs = [
        run,
        open,
        Dialog::confirm_hide(crate::desktop::Place::Computer, "Computer"),
        Dialog::end_program("signal-desktop", 1234),
        log_off(),
        turn_off(),
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
            draw(
                &theme,
                &text,
                &l,
                icon.as_ref(),
                caption.as_ref(),
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
