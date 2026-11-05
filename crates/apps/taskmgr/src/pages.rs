//! The main window's layout, drawing and access tree: the menu bar, the three pages on a tab
//! strip, and the status bar.

use repriseapp::Ctx;
use reprisekit::access::{ROOT, Tree};
use reprisekit::classic::widgets::{BUTTON_H, LINE_H};
use reprisekit::controls::form::{self, Form, Item};
use reprisekit::controls::menubar;
use reprisekit::controls::perfgraph;
use reprisekit::controls::statusbar::{self, Panel, StatusBar};
use reprisekit::controls::tabstrip;
use reprisekit::look::Paint;
use reprisekit::look::widgets::draw_page;
use reprisekit::raster::{Canvas, Rect};

use crate::{Page, TaskMgr, grouped};

pub const CLIENT: (i32, i32) = (420, 460);
pub const MIN: (i32, i32) = (340, 360);

pub const END_TASK: &str = "end_task";
pub const SWITCH_TO: &str = "switch_to";
pub const END_PROCESS: &str = "end_process";
pub const ALL_USERS: &str = "all_users";

/// The window edge to the sheet, and the sheet's edge to what it holds.
const MARGIN: i32 = 6;
const PAD: i32 = 7;
const BUTTON_W: i32 = 75;
const GAP: i32 = 6;
/// A group box's insets round what it holds: the title band above, the edge elsewhere.
const GROUP_TOP: i32 = 16;
const GROUP_EDGE: i32 = 8;
const METER_GROUP_W: i32 = 84;
/// The figures under the graphs: three rows of label and value.
const FIGURES_H: i32 = 3 * LINE_H + 2 * 2 + GROUP_TOP + GROUP_EDGE;
/// Per-core graphs in a row before they wrap.
const CORES_PER_ROW: usize = 8;

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
    pub perf: Option<Perf>,
}

/// The Performance page: per row a meter and its history (each with its group), and the
/// three figure groups under them.
pub struct Perf {
    pub meter_groups: [Rect; 2],
    pub meters: [Rect; 2],
    pub graph_groups: [Rect; 2],
    pub graphs: [Rect; 2],
    pub figures: [Rect; 3],
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
    let perf = (t.page() == Page::Performance).then(|| perf_layout(inner));
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

fn perf_layout(inner: Rect) -> Perf {
    let rows_h = (inner.height - FIGURES_H - PAD).max(2);
    let row_h = (rows_h - PAD) / 2;
    let row = |i: i32| inner.y + i * (row_h + PAD);
    let inside = |g: Rect| {
        Rect::new(
            g.x + GROUP_EDGE,
            g.y + GROUP_TOP,
            (g.width - 2 * GROUP_EDGE).max(1),
            (g.height - GROUP_TOP - GROUP_EDGE).max(1),
        )
    };
    let meter_groups = [0, 1].map(|i| Rect::new(inner.x, row(i), METER_GROUP_W, row_h));
    let graph_x = inner.x + METER_GROUP_W + PAD;
    let graph_groups =
        [0, 1].map(|i| Rect::new(graph_x, row(i), (inner.right() - graph_x).max(1), row_h));
    let fig_y = inner.bottom() - FIGURES_H;
    let fig_w = (inner.width - 2 * PAD) / 3;
    let figures =
        [0, 1, 2].map(|i| Rect::new(inner.x + i * (fig_w + PAD), fig_y, fig_w, FIGURES_H));
    Perf {
        meters: meter_groups.map(inside),
        meter_groups,
        graphs: graph_groups.map(inside),
        graph_groups,
        figures,
    }
}

/// The three figures of a group: label ids, value ids and labels.
const FIGURES: [(&str, [(&str, &str); 3]); 3] = [
    (
        "Totals",
        [
            ("procs", "Processes"),
            ("threads", "Threads"),
            ("programs", "Programs"),
        ],
    ),
    (
        "Physical Memory (MB)",
        [
            ("mem_total", "Total"),
            ("mem_avail", "Available"),
            ("mem_cached", "Cached"),
        ],
    ),
    (
        "Swap (MB)",
        [
            ("swap_total", "Total"),
            ("swap_used", "In use"),
            ("swap_free", "Free"),
        ],
    ),
];

const GROUP_IDS: [&str; 4] = ["cpu_meter", "mem_meter", "cpu_history", "mem_history"];
const FIGURE_GROUPS: [&str; 3] = ["totals", "physical", "swap"];

fn label_id(id: &str) -> &'static str {
    match id {
        "procs" => "procs_label",
        "threads" => "threads_label",
        "programs" => "programs_label",
        "mem_total" => "mem_total_label",
        "mem_avail" => "mem_avail_label",
        "mem_cached" => "mem_cached_label",
        "swap_total" => "swap_total_label",
        "swap_used" => "swap_used_label",
        _ => "swap_free_label",
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
        Page::Performance => {
            let p = lay.perf.as_ref().expect("the Performance layout");
            let titles = [
                "CPU Usage",
                "Memory",
                "CPU Usage History",
                "Memory Usage History",
            ];
            let rects = [
                p.meter_groups[0],
                p.meter_groups[1],
                p.graph_groups[0],
                p.graph_groups[1],
            ];
            let mut items: Vec<Item> = (0..4)
                .map(|i| Item::group(GROUP_IDS[i], rects[i], titles[i]))
                .collect();
            for (g, (title, rows)) in FIGURES.iter().enumerate() {
                let r = p.figures[g];
                items.push(Item::group(FIGURE_GROUPS[g], r, title));
                let w = r.width - 2 * GROUP_EDGE;
                for (i, (id, label)) in rows.iter().enumerate() {
                    let y = r.y + GROUP_TOP + i as i32 * (LINE_H + 2);
                    let x = r.x + GROUP_EDGE;
                    items.push(Item::label_at(label_id(id), x, y, w / 2, label));
                    items.push(Item::value_at(id, x + w / 2, y, w - w / 2, ""));
                }
            }
            items
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
    let key = (t.page(), ctx.size);
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
            let s = &t.snap;
            let mb = |kb: u64| grouped(kb / 1024);
            let values = [
                ("procs", grouped(s.procs.len() as u64)),
                ("threads", grouped(s.threads())),
                ("programs", grouped(s.scopes.len() as u64)),
                ("mem_total", mb(s.mem.total)),
                ("mem_avail", mb(s.mem.available)),
                ("mem_cached", mb(s.mem.cached)),
                ("swap_total", mb(s.mem.swap_total)),
                ("swap_used", mb(s.mem.swap_used())),
                ("swap_free", mb(s.mem.swap_free)),
            ];
            for (id, v) in values {
                f.set_text(id, &v);
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

/// The meters' values: the CPU's load and the memory in use.
fn meter_values(t: &TaskMgr) -> [(f64, String); 2] {
    let cpu = t.cpu.latest().unwrap_or(0.0);
    let m = &t.snap.mem;
    let mem = if m.total == 0 {
        0.0
    } else {
        m.used() as f64 / m.total as f64
    };
    [
        (cpu, format!("{} %", (cpu * 100.0).round())),
        (mem, mem_text(m.used())),
    ]
}

/// Memory in use as a meter writes it: MB under a gigabyte, else GB to one place.
fn mem_text(kb: u64) -> String {
    if kb < 1024 * 1024 {
        format!("{} MB", kb / 1024)
    } else {
        format!("{:.1} GB", kb as f64 / (1024.0 * 1024.0))
    }
}

/// Graph rects for `n` per-core graphs inside `r`, in rows of up to [`CORES_PER_ROW`].
pub fn core_rects(r: Rect, n: usize) -> Vec<Rect> {
    if n == 0 {
        return Vec::new();
    }
    let cols = n.min(CORES_PER_ROW) as i32;
    let rows = n.div_ceil(CORES_PER_ROW) as i32;
    let gap = 3;
    let w = ((r.width - gap * (cols - 1)) / cols).max(1);
    let h = ((r.height - gap * (rows - 1)) / rows).max(1);
    (0..n as i32)
        .map(|i| {
            Rect::new(
                r.x + (i % cols) * (w + gap),
                r.y + (i / cols) * (h + gap),
                w,
                h,
            )
        })
        .collect()
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
        for (i, (f, v)) in meter_values(t).iter().enumerate() {
            perfgraph::draw_meter(c, &ctx.text, &p, perf.meters[i], *f, v, px);
        }
        if t.per_core && !t.cores.is_empty() {
            for (h, r) in t
                .cores
                .iter()
                .zip(core_rects(perf.graphs[0], t.cores.len()))
            {
                perfgraph::draw(c, &p, r, &[h]);
            }
        } else {
            perfgraph::draw(c, &p, perf.graphs[0], &[&t.cpu]);
        }
        perfgraph::draw(c, &p, perf.graphs[1], &[&t.mem, &t.swap]);
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
        let [(_, cpu), (_, mem)] = meter_values(t);
        perfgraph::access(tree, ROOT, perf.meters[0], "CPU Usage", &cpu);
        perfgraph::access(tree, ROOT, perf.meters[1], "Memory", &mem);
        perfgraph::access(tree, ROOT, perf.graphs[0], "CPU Usage History", &cpu);
        perfgraph::access(tree, ROOT, perf.graphs[1], "Memory Usage History", &mem);
    }
    statusbar::access(&status_bar(t), lay.status, ctx.theme.text, tree, ROOT);
    let popup = t.popup.as_ref().map(|m| m.access(tree, ROOT));
    if let Some(f) = menus.or(popup).or(focus) {
        tree.set_focus(f);
    }
}
