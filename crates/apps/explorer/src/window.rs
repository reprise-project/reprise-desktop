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
//! Under a look with glass down the client's top ([`is_shell`]) the window takes the shell
//! layout instead ([`shell_layout`]): one nav row over the glass (Back and Forward, the
//! breadcrumb address, the search field), its 2 px closing line, then the menu bar while Alt
//! or the Layout setting shows it, the command bar, the tabs, the navigation pane, the view and
//! the details pane. Blue's and Classic's toolbar, address band and status bar keys play no part.
//!
//! The view is a reprisekit control that draws a sunken ring round itself,
//! so it is laid out inflated by `FIELD_PAD`: its contents land in place
//! and the ring falls under the chrome painted after it.
use crate::Explorer;
use crate::commands::Command;
use crate::settings::Settings;
use repriseapp::Ctx;
use repriseapp::iconjobs::IconWaits;
use reprisefs::launcher::Launcher;
use reprisefs::listing::Item;
use reprisekit::access::{ROOT, Tree};
use reprisekit::classic::widgets::FIELD_PAD;
use reprisekit::controls::hosted::Chain;
use reprisekit::controls::metrics::ControlMetrics;
use reprisekit::icontheme::IconCache;
use reprisekit::look::Paint;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::tr;
use reprisekit::{
    controls::addressbar, controls::breadcrumb, controls::commandbar, controls::detailspane,
    controls::explorerbar, controls::menubar, controls::navbuttons, controls::rebar,
    controls::searchfield, controls::shellview, controls::statusbar, controls::taskpane,
    controls::toolbar,
};
use reprisekit::{controls::menu, controls::tooltip, controls::treeview};

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
    /// The tab strip under the rebar, with two tabs or more.
    pub tabs: Option<Rect>,
    /// Top row of the line closing the rebar and the tab strip.
    pub etch_y: i32,
    pub bar: Option<explorerbar::Layout>,
    pub tree: Option<Rect>,
    pub tasks: Option<Rect>,
    /// The view's frame, inflated by [`FIELD_PAD`] (see the module doc).
    pub view: Rect,
    pub status: Option<Rect>,
}

/// The nav row's spacing at 100% text: from the client's left edge to the pill, from the
/// recent places drop to the address, from the address to the search field, and from the
/// search field to the right edge.
const NAV_X: i32 = 2;
const NAV_ADDRESS_GAP: i32 = 2;
const NAV_SEARCH_GAP: i32 = 6;
const NAV_RIGHT: i32 = 2;
/// The rows the nav row keeps above and below its fields, together.
const NAV_FIELD_ROOM: i32 = 12;
/// The narrowest the address gets at 100% text: the place's icon, one crumb, the history drop
/// and Refresh.
pub const ADDRESS_MIN: i32 = 120;
/// The menu bar's line under its strip.
const MENU_LINE: i32 = 1;

/// Every rect the shell layout paints, top to bottom.
#[derive(Clone, Debug, PartialEq)]
pub struct ShellLayout {
    /// Over the frame's glass; its closing line runs along under it.
    pub nav: Rect,
    pub back_fwd: Rect,
    /// Back, Forward and the recent places drop at the nav row's left end.
    pub buttons: navbuttons::Layout,
    pub address: Rect,
    pub search: Rect,
    /// The menu bar's row while it shows.
    pub menu: Option<Rect>,
    pub command: Rect,
    pub tabs: Option<Rect>,
    pub nav_pane: Option<Rect>,
    /// The view's frame, inflated by [`FIELD_PAD`] as [`Layout::view`] is.
    pub view: Rect,
    /// The preview pane right of the view, its divider at its left edge.
    pub preview: Option<Rect>,
    pub details: Option<Rect>,
}

/// Whether `theme` takes the shell layout: its frame's glass runs down under the client's top.
pub fn is_shell(theme: &reprisekit::theme::Theme) -> bool {
    reprisekit::look::client_glass(theme, true).is_some()
}

pub fn nav_height(cm: &ControlMetrics) -> i32 {
    (breadcrumb::height(cm.text) + NAV_FIELD_ROOM).max(navbuttons::height(cm.text) + 5)
}

pub fn menu_row_height(cm: &ControlMetrics) -> i32 {
    menubar::bar_h(cm) + MENU_LINE
}

/// The narrowest client the shell layout keeps its nav row whole in.
pub fn shell_min_width(cm: &ControlMetrics) -> i32 {
    let u = cm.text;
    NAV_X
        + navbuttons::width(u)
        + NAV_ADDRESS_GAP
        + u.len(ADDRESS_MIN)
        + NAV_SEARCH_GAP
        + u.len(searchfield::NAV_WIDTH)
        + NAV_RIGHT
}

/// The shell layout of a `size` client: the menu bar shows while `bar_active` (Alt, F10, an
/// access key) or while `settings` keep it, the tab strip is `strip_h` tall, and the navigation
/// pane `settings.nav_width` wide at 100% text, its divider after it.
pub fn shell_layout(
    size: (i32, i32),
    settings: &Settings,
    bar_active: bool,
    strip_h: i32,
    cm: &ControlMetrics,
) -> ShellLayout {
    let client = Rect::new(0, 0, size.0.max(1), size.1.max(1));
    let nav = Rect::new(client.x, client.y, client.width, nav_height(cm));
    let field_h = breadcrumb::height(cm.text);
    let buttons = navbuttons::layout(
        Rect::new(
            client.x + NAV_X,
            nav.y,
            navbuttons::width(cm.text),
            nav.height,
        ),
        cm.text,
    );
    let back_fwd = buttons.frame();
    let row = Rect::new(nav.x, nav.y, nav.width - NAV_RIGHT, nav.height);
    let search = searchfield::nav_layout(row, field_h, cm.text);
    let ax = back_fwd.right() + NAV_ADDRESS_GAP;
    let address = Rect::new(
        ax,
        search.y,
        (search.x - NAV_SEARCH_GAP - ax).max(0),
        field_h,
    );

    let mut top = nav.bottom() + cm.rebar.closing;
    let mut take = |h: i32| {
        let r = Rect::new(client.x, top, client.width, h);
        top = r.bottom();
        r
    };
    let menu = (settings.menu_bar || bar_active).then(|| take(menu_row_height(cm)));
    let command = take(commandbar::height(cm));
    let tabs = (strip_h > 0).then(|| take(strip_h));
    let details = settings.details_pane.then(|| {
        let h = detailspane::height(cm.text).min((client.bottom() - top).max(0));
        Rect::new(client.x, client.bottom() - h, client.width, h)
    });
    let bottom = details.map_or(client.bottom(), |d| d.y);
    let height = (bottom - top).max(0);
    let nav_pane = settings.nav_pane.then(|| {
        let w = cm.text.len(settings.nav_width) + explorerbar::NAV_DIVIDER_W;
        Rect::new(client.x, top, w.min(client.width), height)
    });
    let view_x = nav_pane.map_or(client.x, |r| r.right());
    // A narrow window takes the preview pane's width first, down to none, so the view keeps
    // its least width.
    let room = client.right() - view_x - crate::previewpane::VIEW_MIN_W;
    let preview = settings.preview_pane.then(|| {
        let w = cm.text.len(settings.preview_width) + explorerbar::NAV_DIVIDER_W;
        Rect::new(client.right() - w.min(room), top, w.min(room), height)
    });
    let preview = preview.filter(|r| r.width > explorerbar::NAV_DIVIDER_W);
    let view_right = preview.map_or(client.right(), |r| r.x);
    // The view's ring falls under the panes, the closing line and the details pane.
    let view = Rect::new(
        view_x - FIELD_PAD,
        top - FIELD_PAD,
        (view_right - view_x + 2 * FIELD_PAD).max(0),
        height + 2 * FIELD_PAD,
    );
    ShellLayout {
        nav,
        back_fwd,
        buttons,
        address,
        search,
        menu,
        command,
        tabs,
        nav_pane,
        view,
        preview,
        details,
    }
}

/// The navigation pane's body and divider over `frame`, as [`ShellLayout::nav_pane`] gives it.
pub fn nav_pane_layout(frame: Rect) -> explorerbar::Layout {
    let eb = explorerbar::ExplorerBar {
        title: String::new(),
        width: frame.width - explorerbar::NAV_DIVIDER_W,
        hot_close: false,
        greyed: false,
    };
    explorerbar::nav_layout(&eb, frame)
}

impl Layout {
    /// The rebar layout's rects for a shell layout, which the window's hits and its view read:
    /// the nav row as the rebar, the menu bar's row where it shows (whether it does or not), the
    /// navigation pane as the Folders bar and tree, no toolbar, address band or status bar.
    pub fn of_shell(sl: &ShellLayout, size: (i32, i32), cm: &ControlMetrics) -> Layout {
        let client = Rect::new(0, 0, size.0.max(1), size.1.max(1));
        let etch_y = sl.nav.bottom();
        let menu = sl.menu.unwrap_or(Rect::new(
            client.x,
            etch_y + cm.rebar.closing,
            client.width,
            menu_row_height(cm),
        ));
        let bar = sl.nav_pane.map(nav_pane_layout);
        Layout {
            client,
            rebar: sl.nav,
            menu,
            toolbar: None,
            address: None,
            tabs: sl.tabs,
            etch_y,
            tree: bar.as_ref().map(|b| b.body),
            bar,
            tasks: None,
            view: sl.view,
            status: None,
        }
    }
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
            height: addressbar::height(cm.text),
        });
    }
    rebar::Rebar { bands }
}

pub fn layout(
    size: (i32, i32),
    settings: &Settings,
    bar_width: i32,
    status_h: i32,
    strip_h: i32,
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

    let tabs =
        (strip_h > 0).then(|| Rect::new(client.x, rebar_rect.bottom(), client.width, strip_h));
    let etch_y = rebar_rect.bottom() + strip_h;
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
        let bar_width = cm.text.len(bar_width);
        let frame = Rect::new(
            client.x,
            top,
            (bar_width + explorerbar::SPLITTER_W).min(client.width),
            height,
        );
        let eb = explorerbar::ExplorerBar {
            title: tr!("status-layout-folders"),
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
        tabs,
        etch_y,
        bar,
        tree,
        tasks,
        view,
        status,
    }
}

/// The toolbar's Search field, at the band's right end past its buttons; `None` with the toolbar
/// hidden or too narrow, and under the shell layout, whose field is [`ShellLayout::search`].
pub fn search_field(ex: &Explorer, ctx: &Ctx) -> Option<Rect> {
    let band = ex.layout(ctx).toolbar?;
    let cm = ctx.theme.controls();
    let rects = toolbar::layout(&ex.toolbar, &ctx.text, ctx.theme.fonts.size_px, band, &cm);
    let right = rects.iter().map(|r| r.right()).max().unwrap_or(band.x);
    searchfield::layout(band, right, &cm)
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
    if let Some(shell) = ex.shell_layout(ctx) {
        draw_shell(ex, c, ctx, &lay, &shell);
        draw_overlays(ex, c, ctx, &sl);
        return;
    }

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
    let search = search_field(ex, ctx);
    let draw_field = |c: &mut Canvas| {
        if let Some(al) = &address {
            let focused = ex.focus == crate::Focus::Address;
            let caret = reprisekit::textedit::Caret::new(focused && ex.keyboard, ctx.caret_on);
            addressbar::draw_field(c, &ctx.text, &paint, &ex.address, al, focused, caret, px);
        }
        if let Some(r) = search {
            let focused = ex.focus == crate::Focus::Search;
            let caret = reprisekit::textedit::Caret::new(focused && ex.keyboard, ctx.caret_on);
            let sf = &ex.search_field;
            searchfield::draw(c, &ctx.text, &paint, sf, r, focused, caret, px);
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
    ex.draw_tabs(c, glass, ctx);
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
    draw_overlays(ex, c, ctx, &sl);
}

/// The shell layout's chrome over the view: the nav row clear over the frame's glass and its
/// closing line, the menu bar while it shows, the tabs, and last the address's open list.
fn draw_shell(ex: &Explorer, c: &mut Canvas, ctx: &Ctx, lay: &Layout, sl: &ShellLayout) {
    let px = ctx.theme.fonts.size_px;
    let paint = Paint {
        s: 1,
        ..ctx.theme.paint()
    };
    let cm = ctx.theme.controls();
    c.clear_rect(sl.nav);
    c.report_glass_top(sl.nav.bottom());
    navbuttons::draw(c, &paint, &ex.nav, &sl.buttons);
    let focused = ex.focus == crate::Focus::Address;
    let caret = reprisekit::textedit::Caret::new(focused && ex.keyboard, ctx.caret_on);
    let edit = addressbar::field_layout(&ex.address, sl.address, &cm);
    if focused {
        addressbar::draw_field(c, &ctx.text, &paint, &ex.address, &edit, true, caret, px);
    } else {
        let bl = breadcrumb::layout(&ex.crumbs, sl.address, &ctx.text, px, &cm);
        breadcrumb::draw(c, &ctx.text, &paint, &ex.crumbs, &bl, false, caret, px);
    }
    let searching = ex.focus == crate::Focus::Search;
    let caret = reprisekit::textedit::Caret::new(searching && ex.keyboard, ctx.caret_on);
    let sf = &ex.search_field;
    searchfield::draw_nav(c, &ctx.text, &paint, sf, sl.search, searching, caret, px);
    if let (Some(bl), Some(tree)) = (&lay.bar, lay.tree) {
        explorerbar::draw(c, &ctx.text, &paint, &ex.explorer_bar, bl, px);
        let tl = treeview::layout(&ex.tree, &ctx.text, px, tree, &cm);
        treeview::draw(c, &ctx.text, &paint, &ex.tree, &tl, ex.tree.bars.lit(), px);
    }
    if let Some(r) = sl.details {
        let dl = detailspane::layout(&ex.details, &ctx.text, px, r, &cm);
        detailspane::draw(c, &ctx.text, &paint, &ex.details, &dl, px);
    }
    if let Some(r) = sl.preview {
        ex.draw_pane(c, r, ctx);
    }
    rebar::draw_closing_line(c, &paint, lay.client.x, lay.client.right(), lay.etch_y);
    let cl = ex.cmdbar_layout(sl.command, ctx);
    commandbar::draw(c, &ctx.text, &paint, &ex.cmdbar_shown(), &cl, px);
    ex.draw_tabs(c, false, ctx);
    if sl.menu.is_some() {
        ex.bar.draw(c, &ctx.text, &ctx.theme, &ex.bar_layout(ctx));
    }
    addressbar::draw_list(c, &ctx.text, &paint, &ex.address, &edit, px);
}

/// What lies over every layout's chrome: an open menu, the rename box's balloon and the tip.
fn draw_overlays(ex: &Explorer, c: &mut Canvas, ctx: &Ctx, sl: &shellview::Layout) {
    if let Some(p) = &ex.popup {
        p.draw(c, ctx);
    }
    if let Some((title, body, at, stem)) = rename_balloon(ex, sl, ctx) {
        let b = tooltip::draw_balloon_tip_at(&ctx.theme, &ctx.text, &title, body, c.scale(), stem);
        c.blit_over(at.x, at.y, &b);
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

/// The Rename box's balloon while it is up: its title and body, its rect and its stem.
fn rename_balloon<'a>(
    ex: &'a Explorer,
    sl: &shellview::Layout,
    ctx: &Ctx,
) -> Option<(String, &'a str, Rect, tooltip::Stem)> {
    let (body, (i, _)) = (ex.rename_balloon.as_deref()?, ex.sv.edit.as_ref()?);
    let r = shellview::rename_rect(&ex.sv, sl, *i, &ctx.text, ctx.theme.fonts.size_px)?;
    let title = crate::volumes::label_limit_title();
    let size = tooltip::balloon_size(&ctx.text, &ctx.theme, &title, body);
    let (at, stem) = tooltip::place_at_box(r, size, ctx.size);
    Some((title, body, at, stem))
}

/// The window's access tree, the parts `draw` paints. Focus goes to an open
/// popup, else the menus while they hold the keyboard, else `ex.focus`.
pub fn access(ex: &Explorer, tree: &mut Tree, ctx: &Ctx) {
    if let Some(sl) = ex.shell_layout(ctx) {
        return access_shell(ex, tree, ctx, &sl);
    }
    let px = ctx.theme.fonts.size_px;
    let lay = ex.layout(ctx);
    let menus = ex.bar.access(&ex.bar_layout(ctx), tree, ROOT);
    if let Some(r) = lay.toolbar {
        let rects = toolbar::layout(&ex.toolbar, &ctx.text, px, r, &ctx.theme.controls());
        toolbar::access(&ex.toolbar, r, &rects, &[], tree, ROOT, "toolbar");
    }
    let search = search_field(ex, ctx)
        .map(|r| searchfield::access(&ex.search_field, r, &ctx.text, px, tree, ROOT, "search"));
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
    ex.access_tabs(tree, ctx);
    let (mut folders, mut tasks) = (None, None);
    if let Some(bl) = &lay.bar {
        let pane = explorerbar::access(&ex.explorer_bar, bl, tree, ROOT);
        if let Some(r) = lay.tree {
            let tl = treeview::layout(&ex.tree, &ctx.text, px, r, &ctx.theme.controls());
            folders = Some(treeview::access(
                &ex.tree, &tl, &ctx.text, px, tree, pane, "folders",
            ));
        }
    }
    if let Some(r) = lay.tasks {
        let tl = taskpane::layout(&ex.taskpane, &ctx.text, px, r, &ctx.theme.controls());
        tasks = taskpane::access(&ex.taskpane, &tl, tree, ROOT);
    }
    let sl = ex.view_layout(lay.view, ctx);
    let view = shellview::access(&ex.sv, &sl, &ctx.text, px, tree, ROOT, "view");
    if let Some(r) = lay.status {
        statusbar::access(&ex.status, r, ctx.theme.text, tree, ROOT);
    }
    if let Some((title, body, at, _)) = rename_balloon(ex, &sl, ctx) {
        tooltip::access_balloon_tip("rename", at, &title, body, tree, ROOT);
    }
    let popup = ex
        .popup
        .as_ref()
        .map(|p| p.chain.access(tree, ROOT, "popup"));
    let focus = popup.or(menus).or(match ex.focus {
        crate::Focus::Address => address,
        crate::Focus::Search => search,
        crate::Focus::Tree => folders,
        crate::Focus::Tasks => tasks,
        crate::Focus::Commands | crate::Focus::View => Some(view),
    });
    tree.set_focus(focus.unwrap_or(view));
}

/// [`access`] for the shell layout: the nav row's buttons, the address (its crumbs, or the field
/// while it is edited) and the search field, the menu bar while it shows, the tabs and the view.
fn access_shell(ex: &Explorer, tree: &mut Tree, ctx: &Ctx, sl: &ShellLayout) {
    let px = ctx.theme.fonts.size_px;
    let cm = ctx.theme.controls();
    let menus = sl
        .menu
        .and_then(|_| ex.bar.access(&ex.bar_layout(ctx), tree, ROOT));
    navbuttons::access(&ex.nav, &sl.buttons, tree, ROOT);
    let address = match ex.focus == crate::Focus::Address {
        true => {
            let al = addressbar::field_layout(&ex.address, sl.address, &cm);
            addressbar::access(&ex.address, &al, &ctx.text, px, tree, ROOT)
        }
        false => {
            let bl = breadcrumb::layout(&ex.crumbs, sl.address, &ctx.text, px, &cm);
            breadcrumb::access(&ex.crumbs, &bl, &ctx.text, px, tree, ROOT)
        }
    };
    let sf = &ex.search_field;
    let search = searchfield::access_nav(sf, sl.search, ctx.theme.text, &ctx.text, px, tree, ROOT);
    let cl = ex.cmdbar_layout(sl.command, ctx);
    let commands = commandbar::access(&ex.cmdbar, &cl, tree, ROOT, "commandbar");
    ex.access_tabs(tree, ctx);
    let lay = Layout::of_shell(sl, ctx.size, &cm);
    let mut folders = None;
    if let (Some(bl), Some(r)) = (&lay.bar, lay.tree) {
        let pane = explorerbar::access(&ex.explorer_bar, bl, tree, ROOT);
        let tl = treeview::layout(&ex.tree, &ctx.text, px, r, &cm);
        let name = "navigation";
        folders = Some(treeview::access(
            &ex.tree, &tl, &ctx.text, px, tree, pane, name,
        ));
    }
    if let Some(r) = sl.details {
        let dl = detailspane::layout(&ex.details, &ctx.text, px, r, &cm);
        detailspane::access(&ex.details, &dl, tree, ROOT);
    }
    if let Some(r) = sl.preview {
        ex.access_pane(tree, r);
    }
    let vl = ex.view_layout(sl.view, ctx);
    let view = shellview::access(&ex.sv, &vl, &ctx.text, px, tree, ROOT, "view");
    if let Some((title, body, at, _)) = rename_balloon(ex, &vl, ctx) {
        tooltip::access_balloon_tip("rename", at, &title, body, tree, ROOT);
    }
    let popup = ex
        .popup
        .as_ref()
        .map(|p| p.chain.access(tree, ROOT, "popup"));
    let focus = popup.or(menus).unwrap_or(match ex.focus {
        crate::Focus::Address => address,
        crate::Focus::Search => search,
        crate::Focus::Commands => commands,
        crate::Focus::Tree => folders.unwrap_or(view),
        _ => view,
    });
    tree.set_focus(focus);
}

/// A reader's ScrollIntoView, or Focus on a row out of view: the view's entry or the Folders
/// tree's row at `act`'s list scrolls in. True when one moved.
pub fn reader(ex: &mut Explorer, act: repriseapp::Reader, ctx: &Ctx) -> bool {
    let repriseapp::Reader::Reveal {
        at: (x, y), row, ..
    } = act
    else {
        return false;
    };
    let lay = ex.layout(ctx);
    if lay.view.contains(x, y) {
        let sl = ex.view_layout(lay.view, ctx);
        return shellview::reveal(&mut ex.sv, row, &sl);
    }
    match lay.tree.filter(|r| r.contains(x, y)) {
        Some(r) => {
            let px = ctx.theme.fonts.size_px;
            let tl = treeview::layout(&ex.tree, &ctx.text, px, r, &ctx.theme.controls());
            ex.tree.reveal(row, &tl)
        }
        None => false,
    }
}

/// An open context menu or toolbar drop. The menu bar keeps its own chain
/// inside [`reprisekit::controls::menubar::Bar`].
pub struct Popup {
    menu: crate::commands::Menu,
    tree: menu::MenuTree,
    chain: menu::PopupMenu,
    /// The row icons it wears stand-ins for ([`Popup::land_icons`]).
    waits: IconWaits,
    /// Icon names of rows that carry their own, by command, tried in this order.
    named: Vec<(Command, Vec<String>)>,
    /// The command bar part that opened it, drawn held while it is open.
    pub(crate) from_bar: Option<reprisekit::controls::commandbar::Hit>,
    /// Requested origin.
    #[cfg(test)]
    at: (i32, i32),
}

impl Popup {
    /// Opens `menu` at `at`.
    /// `by_keyboard` (Shift+F10, the Menu key) picks its first row and underlines its keys.
    pub fn new(at: (i32, i32), menu: crate::commands::Menu, ctx: &Ctx, by_keyboard: bool) -> Popup {
        let tree = menu.tree();
        let chain = menu::PopupMenu::open(&tree, at, &popup_ctx(ctx), by_keyboard);
        Popup {
            menu,
            tree,
            chain,
            waits: IconWaits::default(),
            named: Vec::new(),
            from_bar: None,
            #[cfg(test)]
            at,
        }
    }

    pub fn menu(&self) -> &crate::commands::Menu {
        &self.menu
    }

    /// The menu's rows wear stand-ins for the icons `waits` waits on.
    pub fn waiting(self, waits: IconWaits) -> Popup {
        Popup { waits, ..self }
    }

    /// [`Popup::waiting`], each command in `named` wearing the first of its names that lands.
    pub fn named(self, waits: IconWaits, named: Vec<(Command, Vec<String>)>) -> Popup {
        Popup {
            waits,
            named,
            ..self
        }
    }

    pub fn for_bar(self, part: reprisekit::controls::commandbar::Hit) -> Popup {
        Popup {
            from_bar: Some(part),
            ..self
        }
    }

    /// Theme icons landed: the rows that waited wear theirs at `px`, `names` giving each row's icon
    /// names by its command, alternatives tried in [`IconCache::rank`]'s order (none leaves the row
    /// alone). True when they did.
    pub fn land_icons(
        &mut self,
        icons: &mut IconCache,
        px: i32,
        names: impl Fn(&Command) -> Vec<String>,
    ) -> bool {
        if !self.waits.landed(icons) {
            return false;
        }
        let mut waits = IconWaits::default();
        let named = &self.named;
        (self.menu).wear_icons(&mut |cmd| {
            if let Some((_, own)) = named.iter().find(|(c, _)| c == cmd) {
                let own: Vec<&str> = own.iter().map(String::as_str).collect();
                return Some(waits.first(icons, &own, px));
            }
            let names = names(cmd);
            let names: Vec<&str> = names.iter().map(String::as_str).collect();
            let names = icons.rank(&names);
            (!names.is_empty()).then(|| waits.first(icons, &names, px))
        });
        self.waits = waits;
        self.tree = self.menu.tree();
        self.chain.wear_icons(&self.tree);
        true
    }

    /// The icon names the row running `cmd` waits on, when it carries its own.
    #[cfg(test)]
    pub fn named_icons(&self, cmd: &Command) -> Option<&[String]> {
        (self.named.iter())
            .find(|(c, _)| c == cmd)
            .map(|(_, n)| &n[..])
    }

    #[cfg(test)]
    pub fn origin(&self) -> (i32, i32) {
        self.at
    }

    #[cfg(test)]
    pub fn tree(&self) -> &menu::MenuTree {
        &self.tree
    }

    pub fn command(&self, path: &[usize]) -> Command {
        self.menu.command(path)
    }

    pub fn key(&mut self, key: menu::MenuKey, ctx: &Ctx) -> menu::MenuAction {
        self.chain.key(&self.tree, key, &popup_ctx(ctx))
    }

    pub fn click(&mut self, x: i32, y: i32, ctx: &Ctx) -> menu::MenuAction {
        self.chain.click(&self.tree, x, y, &popup_ctx(ctx))
    }

    /// True when the hover changed.
    pub fn motion(&mut self, x: i32, y: i32, ctx: &Ctx) -> bool {
        self.chain.motion(&self.tree, x, y, &popup_ctx(ctx))
    }

    pub fn draw(&self, c: &mut Canvas, ctx: &Ctx) {
        self.chain
            .draw(c, &ctx.theme, &ctx.text, ctx.theme.access_keys);
    }
}

impl Chain for Popup {
    fn chain(&self) -> &menu::PopupMenu {
        &self.chain
    }
    fn chain_mut(&mut self) -> &mut menu::PopupMenu {
        &mut self.chain
    }
}

fn popup_ctx(ctx: &Ctx) -> menu::MenuCtx<'_> {
    menu::MenuCtx {
        text: &ctx.text,
        theme: &ctx.theme,
        px: ctx.theme.fonts.size_px,
        within: ctx.menu_within(),
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
                tr!(
                    "status-panels-selected-object-s-selected",
                    selected = selected
                )
            } else {
                tr!("status-panels-objects", objects = objects)
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
/// /home/u/Desktop Type: Text Document Date Modified: 09/14/2026 3:43 PM".
/// Missing facts are left out.
pub fn bin_status_text(it: &Item, is_12h: bool) -> String {
    let mut facts = Vec::new();
    if let Some(dir) = it.trashed.as_ref().and_then(|t| t.original.parent()) {
        facts.push(tr!(
            "status-bin-original-location",
            dir = dir.display().to_string()
        ));
    }
    facts.push(tr!("status-bin-type", kind = it.kind.as_str()));
    if let Some(secs) = it.modified {
        facts.push(tr!(
            "status-bin-date-modified",
            when = reprisefs::listing::date_text(secs, is_12h)
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
            .map(|dir| tr!("status-shortcut-location", dir = dir.display().to_string())),
    }
}

/// The Details section's lines before wrapping: `true` marks bold, an empty
/// string the gap between groups.
///
/// One item: name, kind, date, size. An image (`dims`) leads with its
/// dimensions, then size, then date. Nothing selected names the folder and its `kind`.
/// Several selected give their count and total size.
pub fn details_text(
    folder: &str,
    kind: crate::place::FolderKind,
    chosen: &[&Item],
    dims: Option<(u32, u32)>,
    is_12h: bool,
) -> Vec<(String, bool)> {
    let mut out: Vec<(String, bool)> = Vec::new();
    match chosen {
        [] => {
            out.push((folder.to_string(), true));
            use crate::place::FolderKind;
            let kind = match kind {
                FolderKind::File => tr!("status-details-text-file-folder"),
                FolderKind::System => tr!("status-details-text-system-folder"),
                FolderKind::Compressed => tr!("status-details-text-compressed-folder"),
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
                tr!("status-details-text-date-modified-when", when = when)
            });
            // Folders are never sized.
            let size =
                (!it.is_folder()).then(|| tr!("status-details-size", size = short_size(it.size)));
            let groups = match dims {
                Some((w, h)) => [
                    Some(tr!(
                        "status-details-text-dimensions-w-x-h",
                        w = w.to_string(),
                        h = h.to_string()
                    )),
                    size,
                    modified,
                ],
                None => [modified, size, None],
            };
            for line in groups.into_iter().flatten() {
                out.push((String::new(), false));
                out.push((line, false));
            }
        }
        many => {
            out.push((
                tr!(
                    "status-details-items-selected",
                    count = many.len().to_string()
                ),
                true,
            ));
            let bytes: u64 = many.iter().filter(|i| !i.is_folder()).map(|i| i.size).sum();
            out.push((String::new(), false));
            out.push((tr!("status-details-size", size = short_size(bytes)), false));
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
        lines.push(tr!(
            "status-infotip-lines-dimensions-w-x-h",
            w = w.to_string(),
            h = h.to_string()
        ));
    }
    lines.push(tr!("status-infotip-type", kind = it.kind.as_str()));
    if dims.is_none()
        && let Some(secs) = it.modified
    {
        lines.push(tr!(
            "status-infotip-date-modified",
            when = reprisefs::listing::date_text(secs, is_12h)
        ));
    }
    if !it.is_folder() {
        lines.push(tr!("status-infotip-size", size = short_size(it.size)));
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
        return vec![tr!("status-folder-tip-text-folder-is-empty")];
    }
    let mut lines = vec![tr!("status-folder-tip-size", size = short_size(size))];
    for (label, names) in [
        (tr!("status-folder-tip-text-folders"), &mut folders),
        (tr!("status-folder-tip-text-files"), &mut files),
    ] {
        if names.is_empty() {
            continue;
        }
        names.sort_by_cached_key(|n| (n.to_lowercase(), n.clone()));
        let more = names.len() > TIP_NAMES;
        names.truncate(TIP_NAMES);
        if more {
            names.push("...".to_string());
        }
        lines.push(tr!(
            "status-folder-tip-names",
            label = label,
            names = names.join(", ")
        ));
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
            0,
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
            0,
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
        let lay = layout(client(), &s, 200, statusbar::height(&cm), 0, &cm);
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
            0,
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
            0,
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
            0,
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
            let lay = layout(client(), &s, 200, statusbar::HEIGHT, 0, &cm);
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
        let lay = layout((792, 566), &s, 200, 23, 0, &blue.look.controls_at_100());
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
        let lay = layout((792, 566), &s, 200, 23, 0, &glass.look.controls_at_100());
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
            0,
            &ControlMetrics::CLASSIC,
        );
        assert_eq!(lay.bar.expect("bar").splitter.x, 300);
        assert_eq!(lay.view.inset(FIELD_PAD).x, 304);
    }

    #[test]
    fn the_folders_bar_width_grows_with_the_text_size() {
        let cm = reprisekit::theme::Theme::default()
            .with_text_size(200)
            .controls();
        let lay = layout((1600, 900), &Settings::default(), 150, 23, 0, &cm);
        let bar = lay.bar.expect("bar");
        assert_eq!(bar.header.width, 300, "150 at 100% draws 300 px at 200%");
        assert_eq!(bar.splitter.x, 300);
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
            program: None,
            hidden: false,
            size,
            modified: Some(1_158_329_100),
            mime: None,
            kind: kind.to_string(),
            drive: None,
            mount_root: false,
            takes_writes: false,
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
