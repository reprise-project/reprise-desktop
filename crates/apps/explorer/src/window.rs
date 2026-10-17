//! Window layout and paint order.
//!
//! Classic, 792x573 client, Folders bar 200 px wide, no status bar:
//!
//! ```text
//! y 0         face, above the rebar
//! y 1..80     the rebar: menu band y 3..24, toolbar y 27..56,
//!             address y 59..80 (see `reprisekit::controls::rebar`)
//! y 81/82     shadow then white across the client, closing the rebar
//!             (`rebar::draw_closing_line`, Classic only)
//! y 83..      the Folders bar (header 85..104, etched line 105/106, tree
//!             from 107) and, from x 204, the view's white
//! x 200..203  the splitter
//! ```
//!
//! Blue's rebar ([`RebarMetrics`]) starts at y 0: menu band y 0..21,
//! toolbar band y 24..61 (38 px), address band y 64..85, no closing line;
//! the bars and the view start at y 86.
//!
//! Glass's starts at y 0 too, clear over the frame's glass: menu y 0..21,
//! toolbar y 24..53, address y 56..77, closing line y 78. The view's Details
//! header starts at y 79, the bars and the view's body at y 80.
//!
//! [`RebarMetrics`]: reprisekit::metrics::RebarMetrics
//!
//! The view is a reprisekit control that draws a sunken ring round itself,
//! so it is laid out inflated by `FIELD_PAD`: its contents land in place
//! and the ring falls under the chrome painted after it.
use crate::Explorer;
use crate::commands::Command;
use crate::settings::Settings;
use accesskit::Role;
use repriseapp::Ctx;
use reprisefs::launcher::Launcher;
use reprisefs::listing::Item;
use reprisekit::access::{self, ROOT, Tree, accesskit};
use reprisekit::classic::widgets::FIELD_PAD;
use reprisekit::controls::metrics::ControlMetrics;
use reprisekit::look::Paint;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::{
    controls::addressbar, controls::explorerbar, controls::menubar, controls::rebar,
    controls::shellview, controls::statusbar, controls::taskpane, controls::toolbar,
};
use reprisekit::{controls::menu, controls::treeview};

/// The task pane's width and its gap to the view at 100% text; both grow with the text.
const TASK_W: i32 = 200;
const TASK_VIEW_GAP: i32 = 10;
/// Fixed widths of the status bar's size and zone panels.
const SIZE_PANEL_W: i32 = 76;
const ZONE_PANEL_W: i32 = 153;

/// Every rect the window is painted from.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub client: Rect,
    pub rebar: Rect,
    /// The menu bar's band; the bar takes its top [`menubar::bar_h`].
    pub menu: Rect,
    pub toolbar: Option<Rect>,
    pub address: Option<Rect>,
    /// Top row of the line closing the rebar.
    pub etch_y: i32,
    pub bar: Option<explorerbar::Layout>,
    pub tree: Option<Rect>,
    pub tasks: Option<Rect>,
    /// The view's frame, inflated by [`FIELD_PAD`] (see the module doc).
    pub view: Rect,
    pub status: Option<Rect>,
}

/// The rebar's bands for `settings`: menu bar, then toolbar and address bar when shown.
pub fn rebar(settings: &Settings, cm: &ControlMetrics) -> rebar::Rebar {
    let mut bands = vec![rebar::Band {
        height: menubar::bar_h(cm) + 2,
    }];
    if settings.toolbars {
        bands.push(rebar::Band {
            height: toolbar::height(true, cm),
        });
    }
    if settings.address_bar {
        bands.push(rebar::Band {
            height: cm.text.len(22),
        });
    }
    rebar::Rebar { bands }
}

pub fn layout(
    size: (i32, i32),
    settings: &Settings,
    bar_width: i32,
    status_h: i32,
    cm: &ControlMetrics,
) -> Layout {
    let client = Rect::new(0, 0, size.0.max(1), size.1.max(1));

    let rb = rebar(settings, cm);
    let rm = &cm.rebar;
    let rebar_rect = Rect::new(
        client.x,
        client.y + rm.top,
        client.width,
        rebar::height(&rb, rm),
    );
    let rects = rebar::layout(&rb, rebar_rect, rm);
    let menu = rects[0];
    let mut next = 1;
    let toolbar = settings.toolbars.then(|| {
        next += 1;
        rects[next - 1]
    });
    let address = settings.address_bar.then(|| {
        next += 1;
        rects[next - 1]
    });

    let etch_y = rebar_rect.bottom();
    let top = etch_y + rm.closing;
    let status = settings
        .status_bar
        .then(|| Rect::new(client.x, client.bottom() - status_h, client.width, status_h));
    let bottom = status.map_or(client.bottom(), |s| s.y);
    let height = (bottom - top).max(0);

    let (bar, tree, tasks, view_x) = if !settings.folders {
        let w = cm.text.len(TASK_W).min(client.width);
        (
            None,
            None,
            Some(Rect::new(client.x, top, w, height)),
            (client.x + w + cm.text.len(TASK_VIEW_GAP)).min(client.right()),
        )
    } else {
        let frame = Rect::new(
            client.x,
            top,
            (bar_width + explorerbar::SPLITTER_W).min(client.width),
            height,
        );
        let eb = explorerbar::ExplorerBar {
            title: "Folders".to_string(),
            width: bar_width,
            hot_close: false,
            greyed: false,
        };
        let lay = explorerbar::layout(&eb, frame, cm);
        // Reaches 1 px left over the ring's highlight column when the look draws one.
        let over = (lay.body.x - frame.x).min(1);
        let tree = Rect::new(
            lay.body.x - over,
            lay.body.y,
            lay.body.width + over,
            lay.body.height,
        );
        let x = lay.splitter.right();
        (Some(lay), Some(tree), None, x)
    };

    // The inflated ring falls outside the client or under the splitter and
    // closing line, both painted after it.
    let view = Rect::new(
        view_x - FIELD_PAD,
        top - FIELD_PAD,
        (client.right() - view_x + 2 * FIELD_PAD).max(0),
        (height + 2 * FIELD_PAD).max(0),
    );

    Layout {
        client,
        rebar: rebar_rect,
        menu,
        toolbar,
        address,
        etch_y,
        bar,
        tree,
        tasks,
        view,
        status,
    }
}

/// Paints the window, bottom layer first; the view goes down before the bar
/// and rebar so their chrome covers its ring.
pub fn draw(ex: &Explorer, c: &mut Canvas, ctx: &Ctx) {
    let cs = &ctx.theme.colors;
    let px = ctx.theme.fonts.size_px;
    let lay = ex.layout(ctx);
    c.fill_rect(lay.client, cs.button_face);

    let sl = ex.view_layout(lay.view, ctx);
    shellview::draw(
        c,
        &ctx.text,
        &Paint {
            s: 1,
            ..ctx.theme.paint()
        },
        &ex.sv,
        &sl,
        ex.sv.bars.lit(),
        px,
    );

    if let Some(bl) = &lay.bar {
        explorerbar::draw(
            c,
            &ctx.text,
            &Paint {
                s: 1,
                ..ctx.theme.paint()
            },
            &ex.explorer_bar,
            bl,
            px,
        );
        if let Some(tree) = lay.tree {
            let tl = treeview::layout(&ex.tree, &ctx.text, px, tree, &ctx.theme.controls());
            treeview::draw(
                c,
                &ctx.text,
                &Paint {
                    s: 1,
                    ..ctx.theme.paint()
                },
                &ex.tree,
                &tl,
                ex.tree.bars.lit(),
                px,
            );
        }
    }
    if let Some(tasks) = lay.tasks {
        let tl = taskpane::layout(&ex.taskpane, &ctx.text, px, tasks, &ctx.theme.controls());
        taskpane::draw(
            c,
            &ctx.text,
            &Paint {
                s: 1,
                ..ctx.theme.paint()
            },
            &ex.taskpane,
            &tl,
            px,
        );
        // Carries the pane's background over the view's left ring.
        let body = lay.view.inset(FIELD_PAD);
        taskpane::draw_gap(
            c,
            &Paint {
                s: 1,
                ..ctx.theme.paint()
            },
            &tl,
            Rect::new(tasks.right(), tasks.y, body.x - tasks.right(), tasks.height),
        );
    }

    let rb = rebar(&ex.settings, &ctx.theme.controls());
    rebar::draw(
        c,
        &Paint {
            s: 1,
            ..ctx.theme.paint()
        },
        &rb,
        lay.rebar,
    );
    let glass_top = ex.glass_top(ctx);
    let glass = glass_top > 0;
    if glass {
        c.clear_rect(lay.rebar);
        c.report_glass_top(glass_top);
    }
    let paint = Paint {
        s: 1,
        ..ctx.theme.paint()
    };
    let glass_ink = glass
        .then(|| reprisekit::look::ink_on_glass(&ctx.theme))
        .flatten();
    let toolbar = lay.toolbar.map(|r| {
        let rects = toolbar::layout(&ex.toolbar, &ctx.text, px, r, &ctx.theme.controls());
        let tb = &ex.toolbar;
        match glass_ink {
            Some(ink) => toolbar::draw_over_glass(c, &ctx.text, &paint, tb, r, &rects, ink, px),
            None => toolbar::draw(c, &ctx.text, &paint, tb, r, &rects, px),
        }
        rects
    });
    let address = lay.address.map(|r| {
        let al = addressbar::layout(&ex.address, &ctx.text, px, r, &ctx.theme.controls());
        addressbar::draw_label_and_go(c, &ctx.text, &paint, &ex.address, &al, glass_ink, px);
        al
    });
    let draw_field = |c: &mut Canvas| {
        if let Some(al) = &address {
            let focused = ex.focus == crate::Focus::Address;
            let caret = reprisekit::textedit::Caret::new(focused, ctx.caret_on);
            addressbar::draw_field(c, &ctx.text, &paint, &ex.address, al, focused, caret, px);
        }
    };
    if !glass {
        draw_field(c);
    }
    let ml = ex.bar_layout(ctx);
    if glass {
        ex.bar.draw_titles_on_glass(c, &ctx.text, &ctx.theme, &ml);
        reprisekit::look::glow_on_glass(c, &ctx.theme, lay.rebar);
        if let Some(rects) = &toolbar {
            toolbar::draw_glass_separators(c, &paint, &ex.toolbar, rects);
        }
        draw_field(c);
    }
    rebar::draw_closing_line(
        c,
        &Paint {
            s: 1,
            ..ctx.theme.paint()
        },
        lay.client.x,
        lay.client.right(),
        lay.etch_y,
    );

    if let Some(r) = lay.status {
        statusbar::draw(
            c,
            &ctx.text,
            &Paint {
                s: 1,
                ..ctx.theme.paint()
            },
            &ex.status,
            r,
            px,
        );
    }

    if glass {
        ex.bar.draw_popup(c, &ctx.text, &ctx.theme);
    } else {
        ex.bar.draw(c, &ctx.text, &ctx.theme, &ml);
    }
    if let Some(r) = lay.address {
        let al = addressbar::layout(&ex.address, &ctx.text, px, r, &ctx.theme.controls());
        addressbar::draw_list(
            c,
            &ctx.text,
            &Paint {
                s: 1,
                ..ctx.theme.paint()
            },
            &ex.address,
            &al,
            px,
        );
    }
    if let Some(p) = &ex.popup {
        p.draw(c, ctx);
    }
    if let Some(tip) = &ex.tip {
        let box_ = if tip.plain {
            let label = tip.lines.join("\n");
            reprisekit::controls::tooltip::draw_tip_at(&ctx.theme, &ctx.text, &label, c.scale())
        } else {
            reprisekit::controls::infotip::draw(&ctx.theme, &ctx.text, &tip.lines, c.scale())
        };
        c.blit_over(tip.at.0, tip.at.1, &box_);
    }
}

/// The window's access tree, the parts `draw` paints. Focus goes to an open
/// popup, else the menus while they hold the keyboard, else `ex.focus`.
pub fn access(ex: &Explorer, tree: &mut Tree, ctx: &Ctx) {
    let px = ctx.theme.fonts.size_px;
    let lay = ex.layout(ctx);
    let menus = ex.bar.access(&ex.bar_layout(ctx), tree, ROOT);
    if let Some(r) = lay.toolbar {
        let rects = toolbar::layout(&ex.toolbar, &ctx.text, px, r, &ctx.theme.controls());
        toolbar::access(&ex.toolbar, r, &rects, tree, ROOT, "toolbar");
    }
    let mut address = None;
    if let Some(r) = lay.address {
        let al = addressbar::layout(&ex.address, &ctx.text, px, r, &ctx.theme.controls());
        address = Some(addressbar::access(
            &ex.address,
            &al,
            &ctx.text,
            px,
            tree,
            ROOT,
        ));
    }
    let mut folders = None;
    if let Some(bl) = &lay.bar {
        let pane = tree.push(
            ROOT,
            "left bar",
            access::named(Role::Pane, bl.frame, &ex.explorer_bar.title),
        );
        if let Some(r) = lay.tree {
            let tl = treeview::layout(&ex.tree, &ctx.text, px, r, &ctx.theme.controls());
            folders = Some(treeview::access(
                &ex.tree, &tl, &ctx.text, px, tree, pane, "folders",
            ));
        }
    }
    if let Some(r) = lay.tasks {
        let tl = taskpane::layout(&ex.taskpane, &ctx.text, px, r, &ctx.theme.controls());
        taskpane::access(&ex.taskpane, &tl, tree, ROOT);
    }
    let sl = ex.view_layout(lay.view, ctx);
    let view = shellview::access(&ex.sv, &sl, &ctx.text, px, tree, ROOT, "view");
    if let Some(r) = lay.status {
        statusbar::access(&ex.status, r, ctx.theme.text, tree, ROOT);
    }
    let popup = ex
        .popup
        .as_ref()
        .map(|p| p.chain.access(tree, ROOT, "popup"));
    let focus = popup.or(menus).or(match ex.focus {
        crate::Focus::Address => address,
        crate::Focus::Tree => folders,
        crate::Focus::View => Some(view),
    });
    tree.set_focus(focus.unwrap_or(view));
}

/// An open context menu or toolbar drop. The menu bar keeps its own chain
/// inside [`reprisekit::controls::menubar::Bar`].
pub struct Popup {
    menu: crate::commands::Menu,
    tree: menu::MenuTree,
    chain: menu::PopupMenu,
    /// Requested origin, before the chain pushed it inside the client.
    #[cfg(test)]
    at: (i32, i32),
}

impl Popup {
    /// Opens `menu` at `at`, pushed back inside `client` if it would overhang.
    /// `by_keyboard` (Shift+F10, the Menu key) picks its first row and underlines its keys.
    pub fn new(
        at: (i32, i32),
        menu: crate::commands::Menu,
        client: Rect,
        ctx: &Ctx,
        by_keyboard: bool,
    ) -> Popup {
        let tree = menu.tree();
        let chain = menu::PopupMenu::open(&tree, at, &popup_ctx(ctx, client), by_keyboard);
        Popup {
            menu,
            tree,
            chain,
            #[cfg(test)]
            at,
        }
    }

    #[cfg(test)]
    pub fn origin(&self) -> (i32, i32) {
        self.at
    }

    pub fn command(&self, path: &[usize]) -> Command {
        self.menu.command(path)
    }

    pub fn key(&mut self, key: menu::MenuKey, ctx: &Ctx, client: Rect) -> menu::MenuAction {
        self.chain.key(&self.tree, key, &popup_ctx(ctx, client))
    }

    pub fn click(&mut self, x: i32, y: i32, ctx: &Ctx, client: Rect) -> menu::MenuAction {
        self.chain.click(&self.tree, x, y, &popup_ctx(ctx, client))
    }

    /// True when the hover changed.
    pub fn motion(&mut self, x: i32, y: i32, ctx: &Ctx, client: Rect) -> bool {
        self.chain.motion(&self.tree, x, y, &popup_ctx(ctx, client))
    }

    pub fn draw(&self, c: &mut Canvas, ctx: &Ctx) {
        self.chain
            .draw(c, &ctx.theme, &ctx.text, ctx.theme.access_keys);
    }
}

fn popup_ctx<'a>(ctx: &'a Ctx, client: Rect) -> menu::MenuCtx<'a> {
    menu::MenuCtx {
        text: &ctx.text,
        theme: &ctx.theme,
        px: ctx.theme.fonts.size_px,
        within: client,
    }
}

/// The status bar's count, size (`None` leaves it blank) and zone panels.
/// The count reads "N objects", or "N object(s) selected" with a selection.
pub fn status_panels(
    objects: usize,
    selected: usize,
    bytes: Option<u64>,
    zone: &str,
    icon: Option<Canvas>,
) -> Vec<statusbar::Panel> {
    vec![
        statusbar::Panel {
            text: if selected > 0 {
                format!("{selected} object(s) selected")
            } else {
                format!("{objects} object{}", if objects == 1 { "" } else { "s" })
            },
            width: 0,
            pad: statusbar::EXPLORER_TEXT_PAD,
            icon: None,
        },
        statusbar::Panel {
            text: bytes.map_or_else(String::new, short_size),
            width: SIZE_PANEL_W,
            pad: statusbar::EXPLORER_TEXT_PAD,
            icon: None,
        },
        statusbar::Panel {
            text: zone.to_string(),
            width: ZONE_PANEL_W,
            pad: statusbar::EXPLORER_TEXT_PAD,
            icon,
        },
    ]
}

/// First status panel for one selected Recycle Bin item: "Original Location:
/// /home/u/Desktop Type: Text Document Date Modified: 9/14/2026 3:43 PM".
/// Missing facts are left out.
pub fn bin_status_text(it: &Item, is_12h: bool) -> String {
    let mut facts = Vec::new();
    if let Some(dir) = it.trashed.as_ref().and_then(|t| t.original.parent()) {
        facts.push(format!("Original Location: {}", dir.display()));
    }
    facts.push(format!("Type: {}", it.kind));
    if let Some(secs) = it.modified {
        facts.push(format!(
            "Date Modified: {}",
            reprisefs::listing::date_text(secs, is_12h)
        ));
    }
    facts.join(" ")
}

/// First status panel for the shortcut at `path`: a web link's address, else
/// "Location: <target folder>". `None` when there is no such folder.
pub fn shortcut_status_text(l: &Launcher, path: &std::path::Path) -> Option<String> {
    match l.kind {
        reprisefs::launcher::Kind::Web => Some(l.target.clone()).filter(|t| !t.is_empty()),
        _ => l
            .folder(path)
            .map(|dir| format!("Location: {}", dir.display())),
    }
}

/// The Details section's lines before wrapping: `true` marks bold, an empty
/// string the gap between groups.
///
/// One item: name, kind, date, size. An image (`dims`) leads with its
/// dimensions, then size, then date. Nothing selected names the folder
/// ("System Folder" when `system`, per [`crate::place::Place::is_system`],
/// else "File Folder"). Several selected give their count and total size.
pub fn details_text(
    folder: &str,
    system: bool,
    chosen: &[&Item],
    dims: Option<(u32, u32)>,
    is_12h: bool,
) -> Vec<(String, bool)> {
    let mut out: Vec<(String, bool)> = Vec::new();
    match chosen {
        [] => {
            out.push((folder.to_string(), true));
            let kind = if system {
                "System Folder"
            } else {
                "File Folder"
            };
            out.push((kind.to_string(), false));
        }
        [it] => {
            // The picture's slot leads, empty: only its gap shows.
            out.push((String::new(), false));
            out.push((it.label().to_string(), true));
            out.push((it.kind.clone(), false));
            let modified = it.modified.map(|secs| {
                let when = reprisefs::props::format_pane_time(secs, is_12h);
                format!("Date Modified: {when}")
            });
            // Folders are never sized.
            let size = (!it.is_folder()).then(|| format!("Size: {}", short_size(it.size)));
            let groups = match dims {
                Some((w, h)) => [Some(format!("Dimensions: {w} x {h}")), size, modified],
                None => [modified, size, None],
            };
            for line in groups.into_iter().flatten() {
                out.push((String::new(), false));
                out.push((line, false));
            }
        }
        many => {
            out.push((format!("{} items selected", many.len()), true));
            let bytes: u64 = many.iter().filter(|i| !i.is_folder()).map(|i| i.size).sum();
            out.push((String::new(), false));
            out.push((format!("Size: {}", short_size(bytes)), false));
        }
    }
    out
}

/// The infotip's lines for `it`; `dims` is the image size once known.
///
/// Image: dimensions, type, size. A plain file drops the dimensions and
/// gains the date. A drive gets kind and date. Folders use [`folder_tip_lines`].
pub fn infotip_lines(it: &Item, dims: Option<(u32, u32)>, is_12h: bool) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some((w, h)) = dims {
        lines.push(format!("Dimensions: {w} x {h}"));
    }
    lines.push(format!("Type: {}", it.kind));
    if dims.is_none()
        && let Some(secs) = it.modified
    {
        lines.push(format!(
            "Date Modified: {}",
            reprisefs::listing::date_text(secs, is_12h)
        ));
    }
    if !it.is_folder() {
        lines.push(format!("Size: {}", short_size(it.size)));
    }
    lines
}

/// Names listed per kind in a folder infotip.
const TIP_NAMES: usize = 3;

/// A folder's infotip: "Size:", then "Folders:" and "Files:" naming the first
/// [`TIP_NAMES`] in name order, ", ..." if more. Empty kinds get no line;
/// an empty folder says "Folder is Empty".
pub fn folder_tip_text(size: u64, mut folders: Vec<String>, mut files: Vec<String>) -> Vec<String> {
    if folders.is_empty() && files.is_empty() {
        return vec!["Folder is Empty".to_string()];
    }
    let mut lines = vec![format!("Size: {}", short_size(size))];
    for (label, names) in [("Folders", &mut folders), ("Files", &mut files)] {
        if names.is_empty() {
            continue;
        }
        names.sort_by_cached_key(|n| (n.to_lowercase(), n.clone()));
        let more = names.len() > TIP_NAMES;
        names.truncate(TIP_NAMES);
        if more {
            names.push("...".to_string());
        }
        lines.push(format!("{label}: {}", names.join(", ")));
    }
    lines
}

/// [`folder_tip_text`] for `dir`; hidden entries (dot names, `.hidden` lists)
/// only with `show_hidden`. Walks the whole tree ([`reprisefs::props::folder_totals_unless`]),
/// so run it on a thread; returns `None` once `stop` is set.
pub fn folder_tip_lines(
    dir: &std::path::Path,
    show_hidden: bool,
    stop: &std::sync::atomic::AtomicBool,
) -> Option<Vec<String>> {
    let (mut folders, mut files) = (Vec::new(), Vec::new());
    let listed = reprisefs::hidden::Listed::read(dir);
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        if !show_hidden && listed.hides(&e.file_name()) {
            continue;
        }
        let name = e.file_name().to_string_lossy().into_owned();
        if e.file_type().is_ok_and(|t| t.is_dir()) {
            folders.push(name);
        } else {
            files.push(name);
        }
    }
    let size = reprisefs::props::folder_totals_unless(dir, stop)?.size;
    Some(folder_tip_text(size, folders, files))
}

pub use reprisefs::props::short_size;

#[cfg(test)]
mod tests {
    use super::*;

    fn client() -> (i32, i32) {
        (792, 573)
    }

    #[test]
    fn the_full_window_with_the_folders_bar_and_no_status_bar() {
        let s = Settings {
            status_bar: false,
            ..Settings::default()
        };
        let lay = layout(
            client(),
            &s,
            200,
            statusbar::HEIGHT,
            &ControlMetrics::CLASSIC,
        );
        assert_eq!(lay.rebar, Rect::new(0, 1, 792, 80));
        assert_eq!(lay.menu, Rect::new(0, 3, 792, 22));
        assert_eq!(lay.toolbar, Some(Rect::new(0, 27, 792, 30)));
        assert_eq!(lay.address, Some(Rect::new(0, 59, 792, 22)));
        assert_eq!(lay.etch_y, 81);
        let bar = lay.bar.expect("folders bar");
        assert_eq!(bar.header, Rect::new(0, 85, 200, 20));
        assert_eq!(bar.splitter, Rect::new(200, 83, 4, 490));
        assert_eq!(lay.tree, Some(Rect::new(1, 107, 197, 464)));
        assert_eq!(lay.view, Rect::new(201, 80, 594, 496));
        let body = lay.view.inset(FIELD_PAD);
        assert_eq!((body.x, body.y), (204, 83));
        assert_eq!((body.right(), body.bottom()), (792, 573));
        assert_eq!(lay.status, None);
        assert!(lay.tasks.is_none());
    }

    #[test]
    fn the_status_bar_takes_its_height_off_the_bottom() {
        let s = Settings {
            status_bar: true,
            ..Settings::default()
        };
        let lay = layout(
            client(),
            &s,
            200,
            statusbar::HEIGHT,
            &ControlMetrics::CLASSIC,
        );
        assert_eq!(lay.status, Some(Rect::new(0, 551, 792, 22)));
        assert_eq!(lay.view.inset(FIELD_PAD).bottom(), 551);
        assert_eq!(lay.bar.expect("bar").frame.bottom(), 551);
    }

    /// At 200% text the menu, toolbar and address bands, the Folders bar's title strip and the
    /// status bar grow with the text.
    #[test]
    fn the_bands_and_bars_grow_with_the_text_size() {
        let s = Settings {
            status_bar: true,
            ..Settings::default()
        };
        let cm = ControlMetrics::CLASSIC.grown(reprisekit::theme::TextUnits::new(22, 11));
        let lay = layout(client(), &s, 200, statusbar::height(&cm), &cm);
        assert_eq!(lay.menu, Rect::new(0, 3, 792, 42));
        assert_eq!(lay.toolbar, Some(Rect::new(0, 47, 792, 43)));
        assert_eq!(lay.address, Some(Rect::new(0, 92, 792, 44)));
        assert_eq!(lay.status, Some(Rect::new(0, 573 - 44, 792, 44)));
        assert_eq!(lay.bar.expect("folders bar").header.height, 40);
    }

    #[test]
    fn dropping_a_band_lifts_everything_under_it() {
        let s = Settings {
            toolbars: false,
            status_bar: false,
            ..Settings::default()
        };
        let lay = layout(
            client(),
            &s,
            200,
            statusbar::HEIGHT,
            &ControlMetrics::CLASSIC,
        );
        assert_eq!(lay.rebar.height, 2 + 22 + 2 + 22);
        assert_eq!(lay.toolbar, None);
        assert_eq!(lay.address, Some(Rect::new(0, 27, 792, 22)));
        assert_eq!(lay.etch_y, 49);
        assert_eq!(lay.bar.expect("bar").frame.y, 51);

        let bare = Settings {
            toolbars: false,
            address_bar: false,
            status_bar: false,
            ..Settings::default()
        };
        let lay = layout(
            client(),
            &bare,
            200,
            statusbar::HEIGHT,
            &ControlMetrics::CLASSIC,
        );
        assert_eq!(lay.rebar.height, 24);
        assert_eq!(lay.address, None);
        assert_eq!(lay.etch_y, 25);
    }

    #[test]
    fn without_the_tree_the_task_pane_takes_the_left_two_hundred() {
        let s = Settings {
            folders: false,
            status_bar: false,
            ..Settings::default()
        };
        let lay = layout(
            client(),
            &s,
            200,
            statusbar::HEIGHT,
            &ControlMetrics::CLASSIC,
        );
        assert!(lay.bar.is_none() && lay.tree.is_none());
        assert_eq!(lay.tasks, Some(Rect::new(0, 83, 200, 490)));
        // Past the pane's 10 px of white.
        assert_eq!(lay.view.inset(FIELD_PAD).x, 210);
    }

    #[test]
    fn the_task_pane_and_its_gap_widen_with_the_text_size() {
        let s = Settings {
            folders: false,
            status_bar: false,
            ..Settings::default()
        };
        for (px, w, view_x) in [(17, 309, 324), (22, 400, 420)] {
            let cm = ControlMetrics::CLASSIC.grown(reprisekit::theme::TextUnits::new(px, 11));
            let lay = layout(client(), &s, 200, statusbar::HEIGHT, &cm);
            assert_eq!(lay.tasks.expect("task pane").width, w, "{px} px text");
            assert_eq!(lay.view.inset(FIELD_PAD).x, view_x, "{px} px text");
        }
    }

    /// Blue's rebar is 86 px with no closing line; the task pane starts under it.
    #[test]
    fn under_blue_the_bands_and_the_pane_start_higher() {
        let blue = reprisekit::interfaces::blue_theme(reprisekit::blue::Variant::Blue);
        let s = Settings {
            folders: false,
            status_bar: true,
            ..Settings::default()
        };
        let lay = layout((792, 566), &s, 200, 23, &blue.look.controls());
        assert_eq!(lay.rebar, Rect::new(0, 0, 792, 86));
        assert_eq!(lay.menu, Rect::new(0, 0, 792, 22));
        assert_eq!(lay.toolbar, Some(Rect::new(0, 24, 792, 38)));
        assert_eq!(lay.address, Some(Rect::new(0, 64, 792, 22)));
        assert_eq!(lay.tasks, Some(Rect::new(0, 86, 200, 457)));
        assert_eq!(lay.view.inset(FIELD_PAD).y, 86);
        assert_eq!(lay.status, Some(Rect::new(0, 543, 792, 23)));
    }

    /// Glass's closing line sits under the rebar, so the task pane starts two rows lower.
    #[test]
    fn under_glass_the_pane_starts_under_the_closing_line() {
        let glass = reprisekit::interfaces::theme_for(reprisekit::interface::Interface::Glass);
        let s = Settings {
            folders: false,
            status_bar: false,
            ..Settings::default()
        };
        let lay = layout((792, 566), &s, 200, 23, &glass.look.controls());
        assert_eq!(lay.rebar, Rect::new(0, 0, 792, 78));
        assert_eq!(lay.menu, Rect::new(0, 0, 792, 22));
        assert_eq!(lay.toolbar, Some(Rect::new(0, 24, 792, 30)));
        assert_eq!(lay.address, Some(Rect::new(0, 56, 792, 22)));
        assert_eq!(lay.etch_y, 78);
        assert_eq!(lay.tasks, Some(Rect::new(0, 80, 200, 486)));
    }

    #[test]
    fn a_dragged_splitter_moves_the_view_with_it() {
        let lay = layout(
            client(),
            &Settings::default(),
            300,
            statusbar::HEIGHT,
            &ControlMetrics::CLASSIC,
        );
        assert_eq!(lay.bar.expect("bar").splitter.x, 300);
        assert_eq!(lay.view.inset(FIELD_PAD).x, 304);
    }

    #[test]
    fn the_status_panels_read_like_explorers() {
        // No directory behind it (Computer): blank size.
        let p = status_panels(1, 0, None, "Computer", None);
        assert_eq!(p[0].text, "1 object");
        assert_eq!(p[1].text, "");
        assert_eq!(p[2].text, "Computer");
        let p = status_panels(3, 0, Some(0), "Computer", None);
        assert_eq!(p[0].text, "3 objects");
        assert_eq!(p[1].text, "0 bytes");
        let p = status_panels(5, 0, Some(114_294_784), "Computer", None);
        assert_eq!(p[0].text, "5 objects");
        assert_eq!(p[1].text, "109 MB");
        let p = status_panels(5, 3, Some(1260), "Computer", None);
        assert_eq!(p[0].text, "3 object(s) selected");
        assert_eq!(p[1].text, "1.23 KB");
    }

    fn item(name: &str, is_dir: bool, kind: &str, size: u64) -> Item {
        Item {
            name: name.to_string(),
            path: std::path::PathBuf::from("/tmp").join(name),
            is_dir,
            is_link: false,
            exec: false,
            program: false,
            hidden: false,
            size,
            modified: Some(1_158_329_100),
            mime: None,
            kind: kind.to_string(),
            drive: None,
            trashed: None,
            shortcut: None,
        }
    }

    #[test]
    fn a_folder_tip_names_three_folders_and_three_files() {
        let names = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            folder_tip_text(
                2_400_000_000,
                names(&["archive", "Albums", "admin", "Code"]),
                names(&["budget.ods", "0.log", "Beach 16.bmp", "clip.mkv"]),
            ),
            [
                "Size: 2.23 GB",
                "Folders: admin, Albums, archive, ...",
                "Files: 0.log, Beach 16.bmp, budget.ods, ...",
            ]
        );
        assert_eq!(
            folder_tip_text(
                11,
                names(&["docs"]),
                names(&["readme.txt", "list.csv", "notes.txt"])
            ),
            [
                "Size: 11 bytes",
                "Folders: docs",
                "Files: list.csv, notes.txt, readme.txt"
            ]
        );
        assert_eq!(
            folder_tip_text(0, names(&["b", "a"]), Vec::new()),
            ["Size: 0 bytes", "Folders: a, b"]
        );
        assert_eq!(
            folder_tip_text(0, Vec::new(), Vec::new()),
            ["Folder is Empty"]
        );
    }

    #[test]
    fn a_folder_tip_reads_the_folder() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let d = dir.path();
        std::fs::create_dir(d.join("docs")).expect("mkdir");
        std::fs::write(d.join("docs").join("a.txt"), "12345").expect("write");
        std::fs::write(d.join("notes.txt"), "123456").expect("write");
        std::fs::write(d.join(".hidden"), "").expect("write");
        let go = std::sync::atomic::AtomicBool::new(false);
        let lines = |show_hidden| folder_tip_lines(d, show_hidden, &go).expect("not stopped");
        assert_eq!(
            lines(false),
            ["Size: 11 bytes", "Folders: docs", "Files: notes.txt"]
        );
        assert_eq!(lines(true)[2], "Files: .hidden, notes.txt");
    }

    #[test]
    fn a_folder_tip_leaves_out_what_hidden_lists() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let d = dir.path();
        std::fs::write(d.join("notes.txt"), "1").expect("write");
        std::fs::write(d.join("secret.txt"), "2").expect("write");
        std::fs::create_dir(d.join("private")).expect("mkdir");
        std::fs::write(d.join(".hidden"), "secret.txt\nprivate\n").expect("write");
        let go = std::sync::atomic::AtomicBool::new(false);
        let lines = |show_hidden| folder_tip_lines(d, show_hidden, &go).expect("not stopped");
        assert_eq!(lines(false)[1..], ["Files: notes.txt"]);
        assert_eq!(
            lines(true)[1..],
            ["Folders: private", "Files: .hidden, notes.txt, secret.txt"]
        );
    }

    #[test]
    fn the_infotip_lines_for_each_kind() {
        let img = item("sample.jpg", false, "JPEG Image", 28_500);
        assert_eq!(
            infotip_lines(&img, Some((800, 600)), true),
            ["Dimensions: 800 x 600", "Type: JPEG Image", "Size: 27.8 KB"]
        );
        // Before the worker answers, the date takes the dimensions' place.
        let lines = infotip_lines(&img, None, true);
        assert_eq!(lines[0], "Type: JPEG Image");
        assert!(lines[1].starts_with("Date Modified: "));
        assert_eq!(lines[2], "Size: 27.8 KB");
        let dir = item("Stuff", true, "File Folder", 4096);
        let lines = infotip_lines(&dir, None, true);
        assert_eq!(lines[0], "Type: File Folder");
        assert!(lines[1].starts_with("Date Modified: "));
        assert_eq!(lines.len(), 2);
    }
}
