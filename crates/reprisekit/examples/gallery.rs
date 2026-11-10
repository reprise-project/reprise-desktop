//! Draws every reprisekit control into PNGs in `target/`. Run with
//! `cargo run -p reprisekit --example gallery`; output names are in `main`.
//! `REPRISE_SCHEME=<name>` (a name from `schemes::names()`, or `all`) picks the
//! colour scheme. `REPRISE_GALLERY_SCALE=<n>` writes only the parts pages, drawn
//! by styles built for scale `n` (see `gallery_parts`), and the Blue tray arrow
//! page. `REPRISE_GALLERY_LOOK=classic` leaves the Blue bands blank and skips
//! the Blue, Glass and taskbar PNGs. `REPRISE_GALLERY_TEXT_SIZE=<percent>` writes
//! only a message box, a property sheet, window controls, panes and the shell
//! view's icon modes in each look at that text size, About's system view and the caption tabs.

#[path = "gallery_checksums/mod.rs"]
mod gallery_checksums;
#[path = "gallery_doctabs/mod.rs"]
mod gallery_doctabs;
#[path = "gallery_fonts/mod.rs"]
mod gallery_fonts;
#[path = "gallery_keys/mod.rs"]
mod gallery_keys;
#[path = "gallery_logon/mod.rs"]
mod gallery_logon;
#[path = "gallery_miniview/mod.rs"]
mod gallery_miniview;
#[path = "gallery_notes/mod.rs"]
mod gallery_notes;
#[path = "gallery_p3/mod.rs"]
mod gallery_p3;
#[path = "gallery_paint/mod.rs"]
mod gallery_paint;
#[path = "gallery_parts/mod.rs"]
mod gallery_parts;
#[path = "gallery_pathcomplete/mod.rs"]
mod gallery_pathcomplete;
#[path = "gallery_perf/mod.rs"]
mod gallery_perf;
#[path = "gallery_power/mod.rs"]
mod gallery_power;
#[path = "gallery_programs/mod.rs"]
mod gallery_programs;
#[path = "gallery_remote_password/mod.rs"]
mod gallery_remote_password;
#[path = "gallery_rename/mod.rs"]
mod gallery_rename;
#[path = "gallery_widgets/mod.rs"]
mod gallery_widgets;

use reprisekit::interface::Interface;
use reprisekit::raster::{Canvas, Rect, write_preview};
use reprisekit::theme::{Color, Theme};
use reprisekit::{interfaces, schemes};

#[path = "gallery/dialogs.rs"]
mod dialogs;
#[path = "gallery/glass.rs"]
mod glass;
#[path = "gallery/glass_controls.rs"]
mod glass_controls;
#[path = "gallery/glass_shell.rs"]
mod glass_shell;
#[path = "gallery/page.rs"]
mod page;
#[path = "gallery/page_files.rs"]
mod page_files;
#[path = "gallery/page_shell.rs"]
mod page_shell;
#[path = "gallery/popups.rs"]
mod popups;
#[path = "gallery/sheets.rs"]
mod sheets;
#[path = "gallery/start.rs"]
mod start;
#[path = "gallery/taskbar.rs"]
mod taskbar;
#[path = "gallery/text_size.rs"]
mod text_size;
#[path = "gallery/views.rs"]
mod views;
#[path = "gallery/windowing.rs"]
mod windowing;
use dialogs::{
    render_about_system, render_copy_progress, render_end_session, render_file_dialog,
    render_label_ellipsis, render_location_prompt, render_monitors, render_stats_panel,
    render_windowed_dialog,
};
use glass::{render_glass, render_glass_dark, render_glass_scale2};
use glass_shell::{render_glass_preview, render_glass_switcher};
use page::render;
use popups::{
    render_glass_clock_popup, render_glass_launcher, render_glass_notify_sheets,
    render_glass_popups, render_launchers, render_now_playing, write_volume_popups,
};
use sheets::{
    render_list_sheet_links, render_list_sheet_search, render_list_sheets, render_long_list_sheets,
    render_tray_customize,
};
use start::{render_blue_start_menus, render_blue_start_panel, render_start_panel};
use taskbar::{
    render_orb_glow, render_quick_more, render_task_attention, render_task_progress,
    render_taskbar_cells, render_taskbar_drag, render_taskbar_edges, render_taskbar_hidden,
    render_taskbar_keys, render_taskbar_top, render_tray_arrow, render_tray_arrow_schemes,
};
use text_size::render_text_size;
use views::{
    render_backgrounds, render_file_preview, render_grid_drag, render_image_view, render_savers,
    render_show_desktop, render_termgrid,
};
use windowing::{render_caption_tabs, render_overview, render_snap_layouts};

/// `t` with its access keys underlined, as a window showing its keyboard cues draws.
fn cued(t: Theme) -> Theme {
    Theme {
        access_keys: true,
        ..t
    }
}

fn main() {
    if let Ok(n) = std::env::var("REPRISE_GALLERY_SCALE") {
        let n: i32 = n.parse().expect("REPRISE_GALLERY_SCALE is a whole number");
        gallery_parts::write_all(n.max(1));
        write_preview(
            &render_tray_arrow_schemes(n.max(1)),
            "reprisekit-gallery-tray-arrow-blue-schemes.png",
        );
        write_preview(
            &render_orb_glow(n.max(1)),
            "reprisekit-gallery-orb-glow-glass.png",
        );
        write_volume_popups(true, n.max(1));
        write_preview(
            &gallery_keys::launcher_windows(n.max(1)),
            &format!("reprisekit-gallery-launcher-windows@{}.png", n.max(1)),
        );
        write_preview(
            &gallery_keys::key_sheet(n.max(1)),
            &format!("reprisekit-gallery-key-sheet@{}.png", n.max(1)),
        );
        write_preview(
            &gallery_power::render(n.max(1)),
            &format!("reprisekit-gallery-power@{}.png", n.max(1)),
        );
        write_preview(
            &gallery_logon::render(n.max(1)),
            &format!("reprisekit-gallery-logon@{}.png", n.max(1)),
        );
        write_preview(
            &gallery_logon::motion::render(n.max(1)),
            &format!("reprisekit-gallery-logon-motion@{}.png", n.max(1)),
        );
        write_preview(
            &gallery_paint::render(n.max(1)),
            &format!("reprisekit-gallery-paint@{}.png", n.max(1)),
        );
        write_preview(
            &gallery_doctabs::render(n.max(1)),
            &format!("reprisekit-gallery-doc-tabs@{}.png", n.max(1)),
        );
        write_preview(
            &gallery_rename::render(n.max(1)),
            &format!("reprisekit-gallery-rename@{}.png", n.max(1)),
        );
        write_preview(
            &gallery_remote_password::render(n.max(1)),
            &format!("reprisekit-gallery-remote-password@{}.png", n.max(1)),
        );
        write_preview(
            &gallery_checksums::render(n.max(1)),
            &format!("reprisekit-gallery-checksums@{}.png", n.max(1)),
        );
        write_preview(
            &gallery_miniview::render(n.max(1)),
            &format!("reprisekit-gallery-mini-view@{}.png", n.max(1)),
        );
        write_preview(
            &gallery_widgets::render(n.max(1)),
            &format!("reprisekit-gallery-desk-widgets@{}.png", n.max(1)),
        );
        write_preview(
            &gallery_notes::render(n.max(1)),
            &format!("reprisekit-gallery-notes@{}.png", n.max(1)),
        );
        write_preview(
            &gallery_fonts::render(n.max(1)),
            &format!("reprisekit-gallery-fonts@{}.png", n.max(1)),
        );
        write_preview(
            &gallery_programs::calculator(n.max(1)),
            &format!("reprisekit-gallery-calculator@{}.png", n.max(1)),
        );
        write_preview(
            &gallery_programs::charmap(n.max(1)),
            &format!("reprisekit-gallery-charmap@{}.png", n.max(1)),
        );
        write_preview(
            &gallery_programs::colordlg(n.max(1)),
            &format!("reprisekit-gallery-colordlg@{}.png", n.max(1)),
        );
        write_preview(
            &gallery_perf::render(n.max(1)),
            &format!("reprisekit-gallery-perf@{}.png", n.max(1)),
        );
        write_preview(
            &render_copy_progress(true, n.max(1)),
            &format!("reprisekit-gallery-copy-progress@{}.png", n.max(1)),
        );
        write_preview(
            &render_task_progress(n.max(1)),
            &format!("reprisekit-gallery-task-progress@{}.png", n.max(1)),
        );
        write_preview(
            &render_task_attention(n.max(1)),
            &format!("reprisekit-gallery-task-attention@{}.png", n.max(1)),
        );
        write_preview(
            &render_termgrid(n.max(1)),
            &format!("reprisekit-gallery-termgrid@{}.png", n.max(1)),
        );
        write_preview(
            &render_file_preview(n.max(1)),
            &format!("reprisekit-gallery-filepreview@{}.png", n.max(1)),
        );
        write_preview(
            &render_image_view(n.max(1)),
            &format!("reprisekit-gallery-imageview@{}.png", n.max(1)),
        );
        write_preview(
            &render_grid_drag(n.max(1)),
            &format!("reprisekit-gallery-grid-drag@{}.png", n.max(1)),
        );
        write_preview(
            &render_windowed_dialog(n.max(1)),
            &format!("reprisekit-gallery-windowed-dialog@{}.png", n.max(1)),
        );
        write_preview(
            &render_label_ellipsis(n.max(1)),
            &format!("reprisekit-gallery-label-ellipsis@{}.png", n.max(1)),
        );
        write_preview(
            &render_blue_start_panel(n.max(1)),
            &format!("reprisekit-gallery-blue-start-menu@{}.png", n.max(1)),
        );
        write_preview(
            &render_blue_start_menus(n.max(1)),
            &format!("reprisekit-gallery-blue-start-menus@{}.png", n.max(1)),
        );
        write_preview(
            &gallery_pathcomplete::render(n.max(1)),
            &format!("reprisekit-gallery-path-complete@{}.png", n.max(1)),
        );
        write_preview(
            &render_list_sheet_search(n.max(1)),
            &format!("reprisekit-gallery-list-sheet-search@{}.png", n.max(1)),
        );
        for mode in [
            reprisekit::glass::Mode::Light,
            reprisekit::glass::Mode::Dark,
        ] {
            let g = reprisekit::glass::Settings {
                mode,
                ..reprisekit::glass::Settings::default()
            };
            let theme = cued(interfaces::theme_with(
                Interface::Glass,
                reprisekit::blue::Variant::default(),
                &g,
            ))
            .scaled(n.max(1));
            let name = if mode == reprisekit::glass::Mode::Dark {
                "dark"
            } else {
                "light"
            };
            write_preview(
                &render_glass_notify_sheets(&theme),
                &format!("reprisekit-gallery-glass-notify-{name}@{}.png", n.max(1)),
            );
        }
        return;
    }
    if let Ok(pct) = std::env::var("REPRISE_GALLERY_TEXT_SIZE") {
        let pct: u32 = pct
            .parse()
            .expect("REPRISE_GALLERY_TEXT_SIZE is a percentage");
        write_preview(
            &render_text_size(pct),
            &format!("reprisekit-gallery-text-size-{pct}.png"),
        );
        write_preview(
            &render_about_system(true, pct),
            &format!("reprisekit-gallery-about-system-{pct}.png"),
        );
        write_preview(
            &render_glass_launcher(pct),
            &format!("reprisekit-gallery-glass-launcher-{pct}.png"),
        );
        write_preview(
            &render_glass_clock_popup(pct),
            &format!("reprisekit-gallery-glass-clock-popup-{pct}.png"),
        );
        write_preview(
            &render_launchers(pct),
            &format!("reprisekit-gallery-launcher-{pct}.png"),
        );
        write_preview(
            &render_caption_tabs(pct),
            &format!("reprisekit-gallery-caption-tabs-{pct}.png"),
        );
        return;
    }
    let with_blue = std::env::var("REPRISE_GALLERY_LOOK").as_deref() != Ok("classic");
    let wanted = std::env::var("REPRISE_SCHEME").ok();
    let names: Vec<Option<&str>> = match wanted.as_deref() {
        None => vec![None],
        Some("all") => std::iter::once(None)
            .chain(schemes::names().map(Some))
            .collect(),
        Some(name) => vec![Some(name)],
    };
    for name in names {
        let mut theme = cued(Theme::default());
        let mut out = "reprisekit-gallery.png".to_string();
        if let Some(name) = name {
            theme.colors = schemes::scheme(name)
                .unwrap_or_else(|| panic!("no scheme {name:?}; see schemes::names()"));
            out = format!("reprisekit-gallery-{}.png", slug(name));
        }
        write_preview(&render(&theme, with_blue), &out);
    }
    write_preview(&render_file_dialog(), "reprisekit-gallery-file-dialog.png");
    write_preview(&render_termgrid(1), "reprisekit-gallery-termgrid.png");
    write_preview(
        &render_file_preview(1),
        "reprisekit-gallery-filepreview.png",
    );
    write_preview(&render_image_view(1), "reprisekit-gallery-imageview.png");
    write_preview(&render_grid_drag(1), "reprisekit-gallery-grid-drag.png");
    write_preview(
        &render_label_ellipsis(1),
        "reprisekit-gallery-label-ellipsis.png",
    );
    write_preview(
        &gallery_pathcomplete::render(1),
        "reprisekit-gallery-path-complete.png",
    );
    write_preview(
        &render_about_system(with_blue, 100),
        "reprisekit-gallery-about-system.png",
    );
    write_preview(
        &render_end_session(with_blue),
        "reprisekit-gallery-end-session.png",
    );
    write_preview(
        &render_monitors(with_blue),
        "reprisekit-gallery-monitors.png",
    );
    write_preview(
        &render_location_prompt(with_blue),
        "reprisekit-gallery-location-prompt.png",
    );
    write_preview(
        &render_copy_progress(with_blue, 1),
        "reprisekit-gallery-copy-progress.png",
    );
    write_preview(
        &render_task_progress(1),
        "reprisekit-gallery-task-progress.png",
    );
    write_preview(
        &render_task_attention(1),
        "reprisekit-gallery-task-attention.png",
    );
    write_preview(
        &render_stats_panel(with_blue),
        "reprisekit-gallery-stats-panel.png",
    );
    write_volume_popups(with_blue, 1);
    write_preview(
        &gallery_doctabs::render(1),
        "reprisekit-gallery-doc-tabs@1.png",
    );
    write_preview(
        &gallery_rename::render(1),
        "reprisekit-gallery-rename@1.png",
    );
    write_preview(
        &gallery_remote_password::render(1),
        "reprisekit-gallery-remote-password@1.png",
    );
    write_preview(
        &gallery_checksums::render(1),
        "reprisekit-gallery-checksums@1.png",
    );
    write_preview(
        &gallery_widgets::render(1),
        "reprisekit-gallery-desk-widgets@1.png",
    );
    write_preview(&gallery_notes::render(1), "reprisekit-gallery-notes@1.png");
    write_preview(&gallery_fonts::render(1), "reprisekit-gallery-fonts@1.png");
    write_preview(
        &gallery_programs::calculator(1),
        "reprisekit-gallery-calculator@1.png",
    );
    write_preview(
        &gallery_programs::charmap(1),
        "reprisekit-gallery-charmap@1.png",
    );
    write_preview(
        &gallery_programs::colordlg(1),
        "reprisekit-gallery-colordlg@1.png",
    );
    write_preview(&gallery_power::render(1), "reprisekit-gallery-power@1.png");
    write_preview(&gallery_logon::render(1), "reprisekit-gallery-logon@1.png");
    write_preview(
        &gallery_logon::motion::render(1),
        "reprisekit-gallery-logon-motion@1.png",
    );
    write_preview(
        &gallery_miniview::render(1),
        "reprisekit-gallery-mini-view@1.png",
    );
    write_preview(&gallery_paint::render(1), "reprisekit-gallery-paint@1.png");
    write_preview(&gallery_perf::render(1), "reprisekit-gallery-perf@1.png");
    if with_blue {
        write_preview(
            &gallery_keys::launcher_windows(1),
            "reprisekit-gallery-launcher-windows@1.png",
        );
        write_preview(
            &gallery_keys::key_sheet(1),
            "reprisekit-gallery-key-sheet@1.png",
        );
        write_preview(
            &render_tray_arrow_schemes(1),
            "reprisekit-gallery-tray-arrow-blue-schemes.png",
        );
        write_preview(&render_orb_glow(1), "reprisekit-gallery-orb-glow-glass.png");
        write_preview(
            &render_windowed_dialog(1),
            "reprisekit-gallery-windowed-dialog.png",
        );
        write_preview(&render_glass(), "reprisekit-gallery-glass.png");
        write_preview(&render_glass_dark(), "reprisekit-gallery-glass-dark.png");
        write_preview(
            &render_glass_scale2(),
            "reprisekit-gallery-glass-scale2.png",
        );
        write_preview(
            &render_show_desktop(),
            "reprisekit-gallery-show-desktop.png",
        );
        write_preview(&render_taskbar_top(), "reprisekit-gallery-taskbar-top.png");
        write_preview(
            &render_taskbar_hidden(),
            "reprisekit-gallery-taskbar-hidden.png",
        );
        write_preview(
            &render_taskbar_edges(),
            "reprisekit-gallery-taskbar-edges.png",
        );
        write_preview(
            &render_taskbar_drag(),
            "reprisekit-gallery-taskbar-drag.png",
        );
        write_preview(
            &render_taskbar_cells(),
            "reprisekit-gallery-taskbar-cells.png",
        );
        write_preview(
            &render_taskbar_keys(),
            "reprisekit-gallery-taskbar-keys.png",
        );
        write_preview(&render_quick_more(), "reprisekit-gallery-quick-more.png");
        write_preview(&render_tray_arrow(), "reprisekit-gallery-tray-arrow.png");
        write_preview(
            &render_glass_preview(),
            "reprisekit-gallery-glass-preview.png",
        );
        write_preview(
            &render_glass_switcher(),
            "reprisekit-gallery-glass-switcher.png",
        );
        write_preview(&render_overview(), "reprisekit-gallery-overview.png");
        write_preview(
            &render_snap_layouts(),
            "reprisekit-gallery-snap-layouts.png",
        );
        write_preview(
            &render_glass_popups(),
            "reprisekit-gallery-glass-popups.png",
        );
        write_preview(&render_start_panel(), "reprisekit-gallery-start-panel.png");
        write_preview(
            &render_blue_start_panel(1),
            "reprisekit-gallery-blue-start-menu.png",
        );
        write_preview(
            &render_blue_start_menus(1),
            "reprisekit-gallery-blue-start-menus.png",
        );
        write_preview(&render_list_sheets(), "reprisekit-gallery-list-sheet.png");
        write_preview(
            &render_list_sheet_links(),
            "reprisekit-gallery-list-sheet-links.png",
        );
        write_preview(
            &render_list_sheet_search(1),
            "reprisekit-gallery-list-sheet-search.png",
        );
        write_preview(
            &render_long_list_sheets(),
            "reprisekit-gallery-list-sheet-scrolled.png",
        );
        write_preview(&render_now_playing(), "reprisekit-gallery-now-playing.png");
        write_preview(
            &render_tray_customize(),
            "reprisekit-gallery-tray-customize.png",
        );
        write_preview(&render_backgrounds(), "reprisekit-gallery-backgrounds.png");
        write_preview(&render_savers(), "reprisekit-gallery-screensavers.png");
        write_preview(
            &render_caption_tabs(100),
            "reprisekit-gallery-caption-tabs.png",
        );
        write_preview(
            &gallery_p3::render(),
            "reprisekit-gallery-blue-variants.png",
        );
    }
}

/// `name` as a file-name part: ASCII alphanumerics lowercased, the rest '-'.
fn slug(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect()
}

/// A `w` x `h` opaque backdrop of diagonal light and dark blue stripes, box blurred three times.
fn blurred_stripes(w: i32, h: i32) -> Canvas {
    let (wu, hu) = (w as usize, h as usize);
    let mut px: Vec<[f32; 3]> = (0..wu * hu)
        .map(|i| {
            let (x, y) = ((i % wu) as i32, (i / wu) as i32);
            if (x + y) / 24 % 2 == 0 {
                [40.0, 90.0, 160.0]
            } else {
                [190.0, 215.0, 240.0]
            }
        })
        .collect();
    let r = 6i32;
    for _ in 0..3 {
        for (dx, dy) in [(1, 0), (0, 1)] {
            let src = px.clone();
            for y in 0..h {
                for x in 0..w {
                    let mut sum = [0.0f32; 3];
                    for d in -r..=r {
                        let sx = (x + d * dx).clamp(0, w - 1) as usize;
                        let sy = (y + d * dy).clamp(0, h - 1) as usize;
                        let p = src[sy * wu + sx];
                        for ch in 0..3 {
                            sum[ch] += p[ch];
                        }
                    }
                    px[y as usize * wu + x as usize] = sum.map(|v| v / (2 * r + 1) as f32);
                }
            }
        }
    }
    let mut out = Canvas::new(w as u32, h as u32);
    for (i, p) in px.iter().enumerate() {
        let [r, g, b] = p.map(|v| v.round() as u8);
        out.fill_rect(
            Rect::new((i % wu) as i32, (i / wu) as i32, 1, 1),
            Color::rgb(r, g, b),
        );
    }
    out
}
