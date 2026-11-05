//! The main window's layout, drawing and access tree: the menu bar, the three pages on a tab
//! strip, and the status bar.

use repriseapp::Ctx;
use reprisekit::access::{ROOT, Tree};
use reprisekit::classic::widgets::BUTTON_H;
use reprisekit::controls::form::{self, Form, Item};
use reprisekit::controls::menubar;
use reprisekit::controls::statusbar::{self, Panel, StatusBar};
use reprisekit::controls::tabstrip;
use reprisekit::look::Paint;
use reprisekit::look::widgets::draw_page;
use reprisekit::raster::{Canvas, Rect};

use crate::{Page, TaskMgr, perf};

pub const CLIENT: (i32, i32) = (590, 460);
pub const MIN: (i32, i32) = (440, 360);

pub const END_TASK: &str = "end_task";
pub const SWITCH_TO: &str = "switch_to";
pub const END_PROCESS: &str = "end_process";
pub const ALL_USERS: &str = "all_users";

/// The window edge to the sheet, and the sheet's edge to what it holds.
const MARGIN: i32 = 6;
const PAD: i32 = 7;
const BUTTON_W: i32 = 75;
const GAP: i32 = 6;

pub struct Layout {
    pub bar: menubar::Layout,
    pub tabs: Vec<Rect>,
    pub page: Rect,
    pub status: Rect,
    /// The list on the open page, if it has one.
    pub list: Option<Rect>,
    /// Where either page's list sits.
    pub list_area: Rect,
    /// The bottom row of buttons on the Applications and Processes pages.
    pub buttons_y: i32,
    pub inner: Rect,
    pub perf: Option<perf::Layout>,
}

pub fn layout(t: &TaskMgr, ctx: &Ctx) -> Layout {
    let cm = ctx.theme.controls();
    let (w, h) = ctx.size;
    let px = ctx.theme.fonts.size_px;
    let bar = (t.bar)
        .layout(&ctx.text, px, Rect::new(0, 0, w, h), &cm)
        .reaching(ctx.menu_within());
    let status_h = statusbar::height(&cm);
    let status = Rect::new(0, h - status_h, w, status_h);
    let top = menubar::bar_h(&cm) + MARGIN;
    let tab_h = ctx.theme.text.len(tabstrip::TAB_H) + 2;
    let tabs = tabstrip::layout(
        &ctx.text,
        px,
        &t.tabs,
        MARGIN,
        top,
        w - 2 * MARGIN,
        ctx.theme.text,
    );
    let page_y = top + tab_h;
    let page = Rect::new(
        MARGIN,
        page_y,
        (w - 2 * MARGIN).max(1),
        (status.y - MARGIN - page_y).max(1),
    );
    let inner = Rect::new(
        page.x + PAD,
        page.y + PAD,
        (page.width - 2 * PAD).max(1),
        (page.height - 2 * PAD).max(1),
    );
    let buttons_y = inner.bottom() - BUTTON_H;
    let list_area = Rect::new(
        inner.x,
        inner.y,
        inner.width,
        (buttons_y - PAD - inner.y).max(1),
    );
    let list = (t.page() != Page::Performance).then_some(list_area);
    let perf =
        (t.page() == Page::Performance).then(|| perf::layout(inner, t.perf.selected().pane()));
    Layout {
        bar,
        tabs,
        page,
        status,
        list,
        list_area,
        buttons_y,
        inner,
        perf,
    }
}

fn build_form(t: &TaskMgr, lay: &Layout) -> Form {
    let inner = lay.inner;
    let by = lay.buttons_y;
    let items = match t.page() {
        Page::Applications => vec![
            Item::button_at(
                END_TASK,
                inner.right() - 2 * BUTTON_W - GAP,
                by,
                BUTTON_W,
                "&End Task",
                true,
            ),
            Item::button_at(
                SWITCH_TO,
                inner.right() - BUTTON_W,
                by,
                BUTTON_W,
                "&Switch To",
                true,
            ),
        ],
        Page::Processes => vec![
            Item::check(ALL_USERS, inner.x, by + 5, "&Show processes from all users"),
            Item::button_at(
                END_PROCESS,
                inner.right() - BUTTON_W - 15,
                by,
                BUTTON_W + 15,
                "&End Process",
                true,
            ),
        ],
        Page::Performance => perf::form_items(lay.perf.as_ref().expect("the Performance layout")),
    };
    Form::new(items)
}

/// Keeps the form on the page and size shown, its buttons live as the selection allows and its
/// figures current.
pub fn follow_form(t: &mut TaskMgr, ctx: &Ctx) {
    if t.refit {
        t.fit_lists(ctx);
    }
    let pane = (t.page() == Page::Performance).then(|| t.perf.selected().pane());
    let key = (t.page(), pane, ctx.size);
    if t.form_for != Some(key) {
        let lay = layout(t, ctx);
        let mut f = build_form(t, &lay);
        f.carry(&t.form);
        t.form = f;
        t.form_for = Some(key);
    }
    let app = t.selected_app().is_some();
    let own = t.selected_proc().is_some_and(|p| p.uid == t.uid);
    let all = t.all_users;
    let f = &mut t.form;
    match key.0 {
        Page::Applications => {
            f.set_enabled(END_TASK, app);
            f.set_enabled(SWITCH_TO, app);
        }
        Page::Processes => {
            f.set_check(ALL_USERS, all);
            f.set_enabled(END_PROCESS, own);
        }
        Page::Performance => {
            for (id, v) in perf::shown(t).values {
                t.form.set_text(id, &v);
            }
        }
    }
}

fn status_bar(t: &TaskMgr) -> StatusBar {
    let s = &t.snap;
    let pct = |used: u64, total: u64| (used * 100).checked_div(total).unwrap_or(0);
    let cpu = (t.cpu.latest().unwrap_or(0.0) * 100.0).round() as u64;
    let panel = |text: String, width: i32| Panel {
        text,
        width,
        pad: statusbar::EXPLORER_TEXT_PAD,
        icon: None,
    };
    StatusBar {
        panels: vec![
            panel(format!("Processes: {}", s.procs.len()), 110),
            panel(format!("CPU Usage: {cpu}%"), 110),
            panel(format!("Memory: {}%", pct(s.mem.used(), s.mem.total)), 0),
        ],
        grip: true,
    }
}

pub fn draw(t: &mut TaskMgr, c: &mut Canvas, ctx: &Ctx) {
    follow_form(t, ctx);
    let t = &*t;
    let lay = layout(t, ctx);
    let px = ctx.theme.fonts.size_px;
    let p = Paint {
        s: 1,
        ..ctx.theme.paint()
    };
    c.fill_rect(c.bounds(), ctx.theme.colors.button_face);
    draw_page(c, &p, lay.page);
    tabstrip::draw(c, &ctx.text, &p, &t.tabs, &lay.tabs, lay.page, false, px);
    if let Some(r) = lay.list {
        let list = match t.page() {
            Page::Applications => &t.apps,
            _ => &t.procs,
        };
        c.fill_rect(r, ctx.theme.colors.window);
        list.draw(c, r, ctx);
    }
    form::draw(c, &ctx.text, &p, &t.form, ctx.theme.access_keys, px);
    if let Some(perf) = &lay.perf {
        perf::draw(t, c, ctx, perf, &p);
    }
    statusbar::draw(c, &ctx.text, &p, &status_bar(t), lay.status, px);
    t.bar.draw(c, &ctx.text, &ctx.theme, &lay.bar);
    if let Some(m) = &t.popup {
        m.draw(c, ctx);
    }
    if let Some(tip) = t.tip.as_ref().filter(|tip| tip.shown)
        && let Some(row) = t.proc_rows.iter().find(|r| r.pid == tip.pid)
        && !row.cmdline.is_empty()
    {
        let tc = reprisekit::controls::tooltip::draw_tip_at(
            &ctx.theme,
            &ctx.text,
            &row.cmdline,
            c.scale(),
        );
        let (w, h) = (tc.width as i32, tc.height as i32);
        let x = (tip.at.0 + 12).min(ctx.size.0 - w).max(0);
        let y = if tip.at.1 + 20 + h <= ctx.size.1 {
            tip.at.1 + 20
        } else {
            (tip.at.1 - h - 4).max(0)
        };
        c.blit_over(x * c.scale(), y * c.scale(), &tc);
    }
}

pub fn access(t: &mut TaskMgr, tree: &mut Tree, ctx: &Ctx) {
    follow_form(t, ctx);
    let lay = layout(t, ctx);
    let px = ctx.theme.fonts.size_px;
    let menus = t.bar.access(&lay.bar, tree, ROOT);
    let tabs = tabstrip::access(&t.tabs, &lay.tabs, tree, ROOT, "tabs");
    let mut focus = tabs.get(t.tabs.open).copied();
    if let Some(r) = lay.list {
        let (list, name) = match t.page() {
            Page::Applications => (&t.apps, "programs"),
            _ => (&t.procs, "processes"),
        };
        let l = list.layout(r, ctx);
        focus = Some(list.view.access(&l, tree, ROOT, name));
    }
    t.form.access(tree, ROOT, &ctx.text, px);
    if let Some(perf) = &lay.perf {
        focus = Some(perf::access(t, tree, ctx, perf));
    }
    statusbar::access(&status_bar(t), lay.status, ctx.theme.text, tree, ROOT);
    let popup = t.popup.as_ref().map(|m| m.access(tree, ROOT));
    if let Some(f) = menus.or(popup).or(focus) {
        tree.set_focus(f);
    }
}
