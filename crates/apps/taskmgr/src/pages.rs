//! The main window's layout, drawing and access tree: the menu bar, the four pages on a tab
//! strip, and the status bar.

use repriseapp::Ctx;
use reprisekit::access::{ROOT, Tree};
use reprisekit::classic::widgets::BUTTON_H;
use reprisekit::controls::form::{self, Form, Item};
use reprisekit::controls::statusbar::{self, Panel, StatusBar};
use reprisekit::controls::tabstrip;
use reprisekit::controls::{listview, menubar};
use reprisekit::look::Paint;
use reprisekit::look::widgets::draw_page;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::theme::TextUnits;
use reprisekit::tr;

use crate::{Page, TaskMgr, perf};

pub const CLIENT: (i32, i32) = (590, 460);
pub const MIN: (i32, i32) = (440, 360);

pub const END_TASK: &str = "end_task";
pub const SWITCH_TO: &str = "switch_to";
pub const NEW_TASK: &str = "new_task";
pub const END_PROCESS: &str = "end_process";
pub const ALL_USERS: &str = "all_users";
pub const SYSTEM: &str = "system_services";
pub const START: &str = "start";
pub const STOP: &str = "stop";
pub const RESTART: &str = "restart";
pub const FILTER_LABEL: &str = "filter_label";
pub const FILTER: &str = "filter";
pub const FAILED: &str = "failed_only";

/// The window edge to the sheet, and the sheet's edge to what it holds.
const MARGIN: i32 = 6;
const PAD: i32 = 7;
const BUTTON_W: i32 = 75;
const GAP: i32 = 6;
/// The Services page's filter row above its list: the label's width, the field's.
const FILTER_H: i32 = 21;
const FILTER_LABEL_W: i32 = 40;
const FILTER_W: i32 = 200;

pub struct Layout {
    pub bar: menubar::Layout,
    pub tabs: Vec<Rect>,
    pub page: Rect,
    pub status: Rect,
    /// The list on the open page, if it has one.
    pub list: Option<Rect>,
    /// Where the Applications and Processes lists sit.
    pub list_area: Rect,
    /// Where the Services list sits, under its filter row.
    pub svc_list: Rect,
    /// The bottom row of buttons on the pages with a list.
    pub buttons_y: i32,
    pub inner: Rect,
    pub perf: Option<perf::Layout>,
    /// How the layout's rows grew with the text; the form is placed at 100% and grows by it.
    pub text: TextUnits,
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
    let buttons_y = inner.bottom() - ctx.theme.text.len(BUTTON_H);
    let list_area = Rect::new(
        inner.x,
        inner.y,
        inner.width,
        (buttons_y - PAD - inner.y).max(1),
    );
    let filter_h = ctx.theme.text.len(FILTER_H) + GAP;
    let svc_list = Rect::new(
        list_area.x,
        list_area.y + filter_h,
        list_area.width,
        (list_area.height - filter_h).max(1),
    );
    let list = match t.page() {
        Page::Performance => None,
        Page::Services => Some(svc_list),
        _ => Some(list_area),
    };
    let perf = (t.page() == Page::Performance)
        .then(|| perf::layout(inner, t.perf.selected().pane(), ctx.theme.text));
    Layout {
        bar,
        tabs,
        page,
        status,
        list,
        list_area,
        svc_list,
        buttons_y,
        inner,
        perf,
        text: ctx.theme.text,
    }
}

fn build_form(t: &TaskMgr, lay: &Layout) -> Form {
    let u = lay.text;
    let inner = u.within_rect(lay.inner);
    let by = u.within(lay.buttons_y);
    let items = match t.page() {
        Page::Applications => vec![
            Item::button_at(
                END_TASK,
                inner.right() - 3 * BUTTON_W - 2 * GAP,
                by,
                BUTTON_W,
                &tr!("end-task"),
                true,
            ),
            Item::button_at(
                SWITCH_TO,
                inner.right() - 2 * BUTTON_W - GAP,
                by,
                BUTTON_W,
                &tr!("switch-to"),
                true,
            ),
            Item::button_at(
                NEW_TASK,
                inner.right() - BUTTON_W,
                by,
                BUTTON_W,
                &tr!("new-task"),
                true,
            ),
        ],
        Page::Processes => vec![
            Item::check(ALL_USERS, inner.x, by + 5, &tr!("all-users")),
            Item::button_at(
                END_PROCESS,
                inner.right() - BUTTON_W - 15,
                by,
                BUTTON_W + 15,
                &tr!("end-process"),
                true,
            ),
        ],
        Page::Performance => {
            perf::form_items(lay.perf.as_ref().expect("the Performance layout"), u)
        }
        Page::Services => {
            let button = |id, i: i32, label: &str| {
                let x = inner.right() - (3 - i) * BUTTON_W - (2 - i) * GAP;
                Item::button_at(id, x, by, BUTTON_W, label, true)
            };
            let field_x = inner.x + FILTER_LABEL_W + 2;
            let field = Rect::new(field_x, inner.y, FILTER_W, FILTER_H);
            vec![
                Item::label_at(
                    FILTER_LABEL,
                    inner.x,
                    inner.y + 4,
                    FILTER_LABEL_W,
                    &tr!("svc-filter"),
                ),
                Item::edit(FILTER, field, &t.svc.filter, true),
                Item::check(
                    FAILED,
                    field.right() + 12,
                    inner.y + 4,
                    &tr!("svc-failed-only"),
                ),
                Item::check(SYSTEM, inner.x, by + 5, &tr!("system-services")),
                button(START, 0, &tr!("start")),
                button(STOP, 1, &tr!("stop")),
                button(RESTART, 2, &tr!("restart")),
            ]
        }
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
    let svc = t.selected_service();
    let may = |verb| svc.is_some_and(|s| t.service_may(s, verb));
    let (start, stop, restart) = (
        may(reprisebus::services::Verb::Start),
        may(reprisebus::services::Verb::Stop),
        may(reprisebus::services::Verb::Restart),
    );
    let (system, failed) = (t.svc.system, t.svc.failed_only);
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
        Page::Services => {
            f.set_check(SYSTEM, system);
            f.set_check(FAILED, failed);
            f.set_enabled(START, start);
            f.set_enabled(STOP, stop);
            f.set_enabled(RESTART, restart);
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
            panel(
                tr!("status-processes", count = s.procs.len().to_string()),
                110,
            ),
            panel(tr!("status-cpu", percent = cpu.to_string()), 110),
            panel(
                tr!(
                    "status-memory",
                    percent = pct(s.mem.used(), s.mem.total).to_string()
                ),
                0,
            ),
        ],
        grip: true,
    }
}

pub fn draw(t: &mut TaskMgr, c: &mut Canvas, ctx: &Ctx) {
    follow_form(t, ctx);
    close_moved_tip(t, ctx);
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
            Page::Services => &t.svc.list,
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
        && let Some(row) = t.proc_rows.iter().find(|r| r.pid == tip.rest.on)
        && !row.cmdline.is_empty()
    {
        c.report_tip(reprisekit::raster::ReportedTip {
            text: row.cmdline.clone(),
            at: tip.rest.at,
        });
    }
}

/// Drops the process tip once the row under the pointer holds another process.
fn close_moved_tip(t: &mut TaskMgr, ctx: &Ctx) {
    let Some(tip) = &t.tip else { return };
    let list = layout(t, ctx).list.filter(|_| t.page() == Page::Processes);
    let under = list.and_then(|r| {
        let (x, y) = tip.pointer;
        match t.procs.view.hit(&t.procs.layout(r, ctx), x, y) {
            listview::Hit::Row(i) => t.proc_rows.get(i).map(|p| p.pid),
            _ => None,
        }
    });
    if under != Some(tip.rest.on) {
        t.tip = None;
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
            Page::Services => (&t.svc.list, "services"),
            _ => (&t.procs, "processes"),
        };
        let l = list.layout(r, ctx);
        focus = Some(list.view.access(&l, tree, ROOT, name));
    }
    let form_focus = t.form.access(tree, ROOT, &ctx.text, px);
    if t.filter_focused() {
        focus = form_focus;
    }
    if let Some(perf) = &lay.perf {
        focus = Some(perf::access(t, tree, ctx, perf));
    }
    statusbar::access(&status_bar(t), lay.status, ctx.theme.text, tree, ROOT);
    let popup = t.popup.as_ref().map(|m| m.access(tree, ROOT));
    if let Some(f) = menus.or(popup).or(focus) {
        tree.set_focus(f);
    }
}
