use super::*;
use crate::procfs::MemInfo;
use crate::scopes::Scope;
use repriseapp::{Extended, MAIN, Mods};
use reprisebus::services::{Failed, Manager, Reread};
use reprisekit::interface::Interface;
use reprisekit::text::TextRenderer;
use reprisekit::theme::Theme;

pub(crate) fn themed_ctx(theme: Theme) -> Ctx {
    Ctx {
        text: TextRenderer::load(&theme.fonts).into(),
        theme,
        icons: reprisekit::icontheme::IconCache::new(reprisekit::icontheme::IconTheme::load(
            "test",
            &[],
        )),
        caret_on: true,
        size: pages::CLIENT,
        maximized: false,
        origin: (0, 0),
        wake: repriseapp::Waker::noop(),
        heads: Default::default(),
        want_tick: Default::default(),
        output_size: None,
        at_pick: false,
        text_size: 100,
    }
}

pub(crate) fn ctx() -> Ctx {
    themed_ctx(Theme::default())
}

const UID: u32 = 1000;

fn entry(id: &str, name: &str, wm_class: Option<&str>) -> AppEntry {
    let class = wm_class.map_or(String::new(), |c| format!("StartupWMClass={c}\n"));
    let text = format!(
        "[Desktop Entry]\nType=Application\nName={name}\nExec={id}\nIcon=text-editor\n{class}"
    );
    reprisefs::apps::parse_desktop(&text, &format!("{id}.desktop").parse().unwrap())
        .expect("an entry")
}

fn proc(pid: i32, name: &str, uid: u32, cpu: f64, mem_kb: u64) -> ProcRow {
    ProcRow {
        pid,
        start: 100 + pid as u64,
        name: name.into(),
        uid,
        user: if uid == UID {
            "u".into()
        } else {
            "root".into()
        },
        cpu,
        mem_kb,
        threads: 2,
        cmdline: format!("/usr/bin/{name} --flag"),
        disk: None,
        gpu: None,
    }
}

/// A window with a task button, its id the pid and no title.
pub(crate) fn window(app_id: &str, pid: u32) -> AppWindow {
    AppWindow {
        id: pid.into(),
        app_id: app_id.into(),
        pid,
        title: String::new(),
        button: true,
    }
}

pub(crate) fn snapshot() -> Snapshot {
    Snapshot {
        procs: vec![
            proc(1, "systemd", 0, 0.0, 12_000),
            proc(40, "notepad", UID, 3.0, 40_960),
            proc(41, "ark", UID, 12.0, 80_000),
            proc(42, "helper", UID, 0.0, 2_048),
        ],
        cpu: Some(0.25),
        cores: vec![0.5, 0.0],
        mem: MemInfo {
            total: 8 << 20,
            available: 6 << 20,
            cached: 1 << 20,
            swap_total: 2 << 20,
            swap_free: 2 << 20,
        },
        scopes: vec![
            Scope {
                unit: "app-reprise-org.kde.ark-1ax2x3.scope".into(),
                app: AppId::from_static("org.kde.ark"),
                pids: vec![41, 42],
            },
            Scope {
                unit: "app-reprise-reprise\\x2dnotepad-2ax1x0.scope".into(),
                app: AppId::from_static("reprise-notepad"),
                pids: vec![40],
            },
            Scope {
                unit: "app-reprise-mystery-3ax1x0.scope".into(),
                app: AppId::from_static("mystery"),
                pids: vec![50],
            },
        ],
        windows: None,
        sandboxed: HashSet::new(),
        devices: Default::default(),
    }
}

pub(crate) fn mgr() -> TaskMgr {
    let entries = vec![
        entry("org.kde.ark", "Ark", None),
        entry("reprise-notepad", "Notepad", Some("reprise.notepad")),
    ];
    TaskMgr::new(None, entries, UID)
}

pub(crate) fn loaded(ctx: &mut Ctx) -> TaskMgr {
    let mut t = mgr();
    t.take(snapshot(), ctx);
    t.event(MAIN, Event::Resized, ctx);
    t
}

pub(crate) fn key(sym: u32) -> Event {
    Event::Key(KeyInput {
        sym,
        utf8: None,
        mods: Mods::default(),
        edit: None,
    })
}

pub(crate) fn click(t: &mut TaskMgr, ctx: &mut Ctx, (x, y): (i32, i32)) {
    t.event(
        MAIN,
        Event::Pointer(PointerInput::Press {
            x,
            y,
            button: BTN_LEFT,
            time: 0,
            mods: Mods::default(),
        }),
        ctx,
    );
    t.event(
        MAIN,
        Event::Pointer(PointerInput::Release {
            x,
            y,
            button: BTN_LEFT,
            mods: Mods::default(),
        }),
        ctx,
    );
}

fn click_release(t: &mut TaskMgr, ctx: &mut Ctx, (x, y): (i32, i32)) {
    t.event(
        MAIN,
        Event::Pointer(PointerInput::Release {
            x,
            y,
            button: BTN_LEFT,
            mods: Mods::default(),
        }),
        ctx,
    );
}

pub(crate) fn centre(r: reprisekit::raster::Rect) -> (i32, i32) {
    (r.x + r.width / 2, r.y + r.height / 2)
}

fn row_point(t: &TaskMgr, ctx: &Ctx, i: usize) -> (i32, i32) {
    let r = pages::layout(t, ctx).list.unwrap();
    let list = match t.page() {
        Page::Applications => &t.apps,
        Page::Services => &t.svc.list,
        _ => &t.procs,
    };
    let l = list.layout(r, ctx);
    (
        l.body.x + 20,
        l.body.y + l.row_h() / 2 + i as i32 * l.row_h(),
    )
}

fn click_row(t: &mut TaskMgr, ctx: &mut Ctx, i: usize) {
    let at = row_point(t, ctx, i);
    click(t, ctx, at);
}

pub(crate) fn open(t: &mut TaskMgr, ctx: &mut Ctx, page: Page) {
    let i = Page::ALL.iter().position(|p| *p == page).unwrap();
    let tabs = pages::layout(t, ctx).tabs;
    click(t, ctx, centre(tabs[i]));
    assert_eq!(t.page(), page);
}

#[test]
fn each_app_scope_is_one_program_named_by_its_desktop_entry() {
    let mut ctx = ctx();
    let t = loaded(&mut ctx);
    let names: Vec<&str> = t.app_rows.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(
        names,
        ["Ark", "Notepad", "mystery"],
        "an entry-less scope shows its id"
    );
    assert_eq!(t.apps.view.rows[0].cells, ["Ark", "Running"]);
}

#[test]
fn a_window_belongs_to_the_program_its_entry_or_id_names() {
    let entries = vec![entry("reprise-notepad", "Notepad", Some("reprise.notepad"))];
    assert!(
        window_of(
            &AppId::from_static("reprise-notepad"),
            &entries,
            "reprise.notepad"
        ),
        "by StartupWMClass"
    );
    assert!(
        window_of(&AppId::from_static("org.kde.Ark"), &entries, "org.kde.ark"),
        "by id, any case"
    );
    assert!(!window_of(
        &AppId::from_static("reprise-notepad"),
        &entries,
        "firefox"
    ));
}

#[test]
fn only_your_own_processes_show_until_all_users_is_ticked() {
    let mut ctx = ctx();
    let mut t = loaded(&mut ctx);
    open(&mut t, &mut ctx, Page::Processes);
    assert_eq!(t.proc_rows.len(), 3);
    let check = t.form.rect(pages::ALL_USERS).unwrap();
    click(&mut t, &mut ctx, (check.x + 5, check.y + 5));
    assert!(t.all_users);
    assert_eq!(t.proc_rows.len(), 4);
    let sys = t.proc_rows.iter().position(|p| p.uid == 0).unwrap();
    click_row(&mut t, &mut ctx, sys);
    pages::follow_form(&mut t, &ctx);
    assert!(
        !enabled(&t, pages::END_PROCESS),
        "another user's process cannot be ended"
    );
    t.event(MAIN, key(keysym::DELETE), &mut ctx);
    assert!(t.asks.is_empty());
}

fn enabled(t: &TaskMgr, id: &str) -> bool {
    match &t.form.item(id).unwrap().control {
        form::Control::Button { enabled, .. } => *enabled,
        c => panic!("{id} is a button, not {c:?}"),
    }
}

#[test]
fn processes_sort_by_cpu_first_and_a_column_title_sorts_by_it() {
    let mut ctx = ctx();
    let mut t = loaded(&mut ctx);
    open(&mut t, &mut ctx, Page::Processes);
    let names = |t: &TaskMgr| {
        t.proc_rows
            .iter()
            .map(|p| p.name.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(names(&t), ["ark", "notepad", "helper"]);
    let r = pages::layout(&t, &ctx).list.unwrap();
    let l = t.procs.layout(r, &ctx);
    let header_y = l.body.y - 5;
    click(&mut t, &mut ctx, (r.x + 20, header_y));
    assert_eq!(
        names(&t),
        ["ark", "helper", "notepad"],
        "Image Name ascending"
    );
    assert_eq!(t.procs.view.sort, Some((0, true)));
    click(&mut t, &mut ctx, (r.x + 20, header_y));
    assert_eq!(names(&t), ["notepad", "helper", "ark"], "and descending");
    assert_eq!(
        t.procs.view.rows[0].cells,
        ["notepad", "40", "u", "03", "40,960 K"]
    );
}

#[test]
fn a_selection_follows_its_process_across_samples() {
    let mut ctx = ctx();
    let mut t = loaded(&mut ctx);
    open(&mut t, &mut ctx, Page::Processes);
    click_row(&mut t, &mut ctx, 1);
    assert_eq!(t.selected_proc().unwrap().pid, 40);
    let mut s = snapshot();
    s.procs[1].cpu = 50.0;
    t.take(s, &mut ctx);
    assert_eq!(t.proc_rows[0].pid, 40, "it rose to the top");
    assert_eq!(t.selected_proc().unwrap().pid, 40, "and stayed selected");
}

#[test]
fn end_process_asks_first_and_offers_end_now_once_it_outlives_the_wait() {
    let mut ctx = ctx();
    let mut t = loaded(&mut ctx);
    t.open_process = |_, _| reprisebus::pidfd::open(std::process::id());
    open(&mut t, &mut ctx, Page::Processes);
    click_row(&mut t, &mut ctx, 2);
    t.event(MAIN, key(keysym::DELETE), &mut ctx);
    let Some((_, Ask::EndProcess { pid, start, .. })) = t.asks.up() else {
        panic!("Delete asks before ending");
    };
    assert_eq!((*pid, *start), (42, 142));
    assert_eq!(t.children().len(), 1, "the warning opens as a window");
    t.asks.close();
    t.wait_for(Target::Process(42, 142), "helper".into());
    t.take(snapshot(), &mut ctx);
    assert!(t.asks.is_empty(), "it has a few seconds to end");
    t.ending[0].since -= END_WAIT;
    t.take(snapshot(), &mut ctx);
    assert!(matches!(
        t.asks.up(),
        Some((_, Ask::EndNow(Target::Process(42, 142))))
    ));
    t.asks.close();
    t.answered(Ask::EndNow(Target::Process(42, 142)), Some(1), &ctx);
    assert!(t.ending.is_empty(), "Cancel stops waiting");
    let mut s = snapshot();
    s.procs[3].start = 999;
    t.wait_for(Target::Process(42, 142), "helper".into());
    t.take(s, &mut ctx);
    assert!(
        t.ending.is_empty(),
        "a reused pid is another process: the first one ended"
    );
}

#[test]
fn end_process_asks_nothing_for_a_process_gone_since_the_sample() {
    let mut ctx = ctx();
    let mut t = loaded(&mut ctx);
    t.open_process = |_, _| Err(std::io::Error::from_raw_os_error(libc::ESRCH));
    open(&mut t, &mut ctx, Page::Processes);
    click_row(&mut t, &mut ctx, 2);
    t.event(MAIN, key(keysym::DELETE), &mut ctx);
    assert!(
        t.asks.is_empty(),
        "no box for a pid that names another process now"
    );
}

#[test]
fn an_ended_program_is_no_longer_waited_on() {
    let mut ctx = ctx();
    let mut t = loaded(&mut ctx);
    let unit = t.app_rows[0].unit.clone().unwrap();
    t.wait_for(Target::Scope(unit), "Ark".into());
    t.ending[0].since -= END_WAIT;
    let mut s = snapshot();
    s.scopes.remove(0);
    t.take(s, &mut ctx);
    assert!(t.ending.is_empty());
    assert!(t.asks.is_empty());
}

#[test]
fn a_question_about_a_process_that_exits_is_withdrawn_and_the_next_comes_up() {
    let mut ctx = ctx();
    let mut t = loaded(&mut ctx);
    t.open_process = |_, _| reprisebus::pidfd::open(std::process::id());
    open(&mut t, &mut ctx, Page::Processes);
    click_row(&mut t, &mut ctx, 2);
    t.event(MAIN, key(keysym::DELETE), &mut ctx);
    let unit = t.app_rows[0].unit.clone().unwrap();
    t.wait_for(Target::Scope(unit.clone()), "Ark".into());
    t.ending[0].since -= END_WAIT;
    t.take(snapshot(), &mut ctx);
    assert!(
        matches!(t.asks.up(), Some((_, Ask::EndProcess { pid: 42, .. }))),
        "End Now waits behind the End Process question"
    );
    let first = t.asks.win();
    let mut s = snapshot();
    s.procs.remove(3);
    t.take(s, &mut ctx);
    assert!(
        matches!(t.asks.up(), Some((_, Ask::EndNow(Target::Scope(u)))) if *u == unit),
        "the question about pid 42 went with it, and End Now is up"
    );
    assert_eq!(t.asks.len(), 1);
    assert_ne!(t.asks.win(), first, "in a window of its own");
}

#[test]
fn the_buttons_follow_the_selection() {
    let mut ctx = ctx();
    let mut t = loaded(&mut ctx);
    pages::follow_form(&mut t, &ctx);
    assert!(!enabled(&t, pages::END_TASK));
    click_row(&mut t, &mut ctx, 0);
    pages::follow_form(&mut t, &ctx);
    assert!(enabled(&t, pages::END_TASK));
    assert!(enabled(&t, pages::SWITCH_TO));
}

#[derive(Debug, PartialEq)]
enum Sent {
    Every(Option<Duration>),
    Now,
    Disk(bool),
    Gpu(bool),
}

fn sent(rx: &std::sync::mpsc::Receiver<worker::Cmd>) -> Vec<Sent> {
    rx.try_iter()
        .map(|c| match c {
            worker::Cmd::Every(d) => Sent::Every(d),
            worker::Cmd::Now => Sent::Now,
            worker::Cmd::Disk(on) => Sent::Disk(on),
            worker::Cmd::Gpu(on) => Sent::Gpu(on),
        })
        .collect()
}

#[test]
fn sampling_stops_while_the_window_is_suspended_and_follows_the_update_speed() {
    let mut ctx = ctx();
    let mut t = mgr();
    let (tx, cmds) = std::sync::mpsc::channel();
    let (_out, rx) = std::sync::mpsc::channel();
    t.worker = Some(worker::Handle { tx, rx });
    t.event(MAIN, Event::Suspended(true), &mut ctx);
    assert_eq!(sent(&cmds), [Sent::Every(None)]);
    t.act(Act::Speed(Speed::High));
    assert_eq!(sent(&cmds), [Sent::Every(None)], "still suspended");
    t.event(MAIN, Event::Suspended(false), &mut ctx);
    assert_eq!(sent(&cmds), [Sent::Every(Some(Duration::from_millis(500)))]);
    assert_eq!(t.menu_act(2, &[1, 3]), Some(Act::Speed(Speed::Paused)));
    t.act(Act::Speed(Speed::Paused));
    t.event(MAIN, key(keysym::F5), &mut ctx);
    assert_eq!(
        sent(&cmds),
        [Sent::Every(None), Sent::Now],
        "F5 samples while paused"
    );
}

#[test]
fn the_view_menu_marks_the_speed_and_graph_picked() {
    let mut t = mgr();
    let marked = |t: &TaskMgr, path: &[usize]| -> Vec<bool> {
        let tree = t.bar.menus[2].at(path).unwrap();
        tree.items()
            .iter()
            .map(|i| {
                matches!(
                    i,
                    MenuItem::Entry {
                        checked: true,
                        radio: true,
                        ..
                    }
                )
            })
            .collect()
    };
    assert_eq!(
        marked(&t, &[1]),
        [false, true, false, false],
        "Normal at start"
    );
    t.act(Act::Speed(Speed::Low));
    t.act(Act::PerCore(true));
    assert_eq!(marked(&t, &[1]), [false, false, true, false]);
    assert_eq!(marked(&t, &[3]), [false, true]);
}

/// The right edge of each Processes column.
fn edges(t: &TaskMgr, l: &listview::Layout) -> Vec<i32> {
    let mut x = l.header.x;
    (t.procs.view.columns.iter())
        .map(|c| {
            x += l.col_w(c);
            x
        })
        .collect()
}

fn titles(t: &TaskMgr) -> Vec<String> {
    t.procs
        .view
        .columns
        .iter()
        .map(|c| c.title.clone())
        .collect()
}

/// Ticks `keys` in Select Columns and presses OK.
fn pick_columns(t: &mut TaskMgr, ctx: &mut Ctx, keys: &[&str]) {
    t.act(t.menu_act(2, &[4]).expect("Select Columns..."));
    let f = t.pick.as_mut().expect("Select Columns opens");
    for k in keys {
        f.set_check(k, true);
    }
    t.event(PICK, key(keysym::RETURN), ctx);
    assert!(t.pick.is_none(), "OK closes it");
    pages::follow_form(t, ctx);
}

#[test]
fn select_columns_opens_from_the_processes_page_only() {
    let mut ctx = ctx();
    let mut t = loaded(&mut ctx);
    let item = |t: &TaskMgr| t.bar.menus[2].items()[4].clone();
    assert!(matches!(item(&t), MenuItem::Entry { enabled: false, .. }));
    open(&mut t, &mut ctx, Page::Processes);
    assert!(matches!(item(&t), MenuItem::Entry { enabled: true, .. }));
    t.act(Act::SelectColumns);
    let kids = t.children();
    assert_eq!(kids.len(), 1);
    assert!(kids[0].modal && kids[0].title == "Select Columns");
    let f = t.pick.as_ref().unwrap();
    assert_eq!(f.checked("name"), Some(true));
    assert_eq!(f.checked("command_line"), Some(false));
    t.event(PICK, key(keysym::ESCAPE), &mut ctx);
    assert!(t.pick.is_none(), "Escape cancels");
    assert_eq!(titles(&t).len(), 5);
}

#[test]
fn picked_columns_show_in_table_order_with_command_line_last() {
    let mut ctx = ctx();
    let mut t = loaded(&mut ctx);
    ctx.size = (800, 460);
    t.event(MAIN, Event::Resized, &mut ctx);
    open(&mut t, &mut ctx, Page::Processes);
    let mut s = snapshot();
    s.procs[1].cmdline.clear();
    t.take(s, &mut ctx);
    pick_columns(&mut t, &mut ctx, &["command_line", "threads"]);
    assert_eq!(
        titles(&t),
        [
            "Image Name",
            "PID",
            "User Name",
            "CPU",
            "Mem Usage",
            "Threads",
            "Command Line"
        ]
    );
    assert_eq!(
        t.procs.view.rows[0].cells,
        [
            "ark",
            "41",
            "u",
            "12",
            "80,000 K",
            "2",
            "/usr/bin/ark --flag"
        ]
    );
    assert_eq!(t.procs.view.rows[1].cells[6], "[notepad]");
    let r = pages::layout(&t, &ctx).list.unwrap();
    let l = t.procs.layout(r, &ctx);
    assert!(
        l.hscroll.is_none() && t.procs.view.columns[6].width > 200,
        "Command Line fills what the others leave"
    );
    let edges = edges(&t, &l);
    click(&mut t, &mut ctx, (edges[5] + 20, l.header.y + 4));
    assert_eq!(t.procs.view.sort, Some((6, true)), "it sorts");
    let names: Vec<&str> = t.proc_rows.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["ark", "helper", "notepad"]);
}

#[test]
fn the_disk_column_turns_the_io_reading_on_and_off() {
    let mut ctx = ctx();
    let mut t = loaded(&mut ctx);
    let (tx, cmds) = std::sync::mpsc::channel();
    let (_out, rx) = std::sync::mpsc::channel();
    t.worker = Some(worker::Handle { tx, rx });
    open(&mut t, &mut ctx, Page::Processes);
    pick_columns(&mut t, &mut ctx, &["disk"]);
    assert_eq!(sent(&cmds), [Sent::Disk(true), Sent::Gpu(false)]);
    let mut s = snapshot();
    s.procs[2].disk = Some(2048);
    t.take(s, &mut ctx);
    assert_eq!(t.procs.view.rows[0].cells[5], "2 KB/s");
    assert_eq!(t.procs.view.rows[1].cells[5], "", "blank where unreadable");
    t.set_columns(columns::defaults());
    assert_eq!(sent(&cmds), [Sent::Disk(false), Sent::Gpu(false)]);
}

#[test]
fn the_gpu_column_turns_the_gpu_client_reading_on_and_off() {
    let mut ctx = ctx();
    let mut t = loaded(&mut ctx);
    let (tx, cmds) = std::sync::mpsc::channel();
    let (_out, rx) = std::sync::mpsc::channel();
    t.worker = Some(worker::Handle { tx, rx });
    open(&mut t, &mut ctx, Page::Processes);
    pick_columns(&mut t, &mut ctx, &["gpu"]);
    assert_eq!(sent(&cmds), [Sent::Disk(false), Sent::Gpu(true)]);
    let mut s = snapshot();
    s.procs[2].gpu = Some(0.25);
    t.take(s, &mut ctx);
    assert_eq!(t.procs.view.columns[5].title, "GPU");
    assert_eq!(t.procs.view.rows[0].cells[5], "25");
    assert_eq!(t.procs.view.rows[1].cells[5], "", "blank without a client");
    t.set_columns(columns::defaults());
    assert_eq!(sent(&cmds), [Sent::Disk(false), Sent::Gpu(false)]);
}

#[test]
fn hiding_the_sorted_column_sorts_by_image_name() {
    let mut ctx = ctx();
    let mut t = loaded(&mut ctx);
    t.set_columns(vec![ProcColumn::Pid]);
    assert_eq!(titles(&t), ["Image Name", "PID"]);
    assert_eq!(t.procs.view.sort, Some((0, true)));
}

#[test]
fn columns_widths_and_view_picks_persist_in_the_state_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("taskmgr.toml");
    let mut ctx = ctx();
    let mut t = loaded(&mut ctx);
    t.restore(path.clone());
    assert!(!path.exists(), "nothing written until something changes");
    open(&mut t, &mut ctx, Page::Processes);
    pick_columns(&mut t, &mut ctx, &["command_line"]);
    t.act(Act::Speed(Speed::Low));
    t.act(Act::PerCore(true));
    let r = pages::layout(&t, &ctx).list.unwrap();
    let l = t.procs.layout(r, &ctx);
    let edge = edges(&t, &l)[1];
    let y = l.header.y + 4;
    t.event(
        MAIN,
        Event::Pointer(PointerInput::Press {
            x: edge,
            y,
            button: BTN_LEFT,
            time: 0,
            mods: Mods::default(),
        }),
        &mut ctx,
    );
    t.event(
        MAIN,
        Event::Pointer(PointerInput::Motion { x: edge + 30, y }),
        &mut ctx,
    );
    click_release(&mut t, &mut ctx, (edge + 30, y));
    assert_eq!(
        t.procs.view.columns[1].width, 78,
        "the divider drag widened PID"
    );
    let mut again = loaded(&mut ctx);
    again.restore(path.clone());
    assert_eq!(titles(&again), titles(&t));
    assert_eq!(again.procs.view.columns[1].width, 78);
    assert_eq!((again.speed, again.per_core), (Speed::Low, true));
    let kept = std::fs::read_to_string(&path).unwrap();
    assert!(
        kept.contains("command_line") && kept.contains("pid = 78"),
        "{kept}"
    );
}

fn statuses(t: &TaskMgr) -> Vec<String> {
    t.apps
        .view
        .rows
        .iter()
        .map(|r| r.cells[1].clone())
        .collect()
}

#[test]
fn a_program_whose_window_stopped_answering_reads_not_responding() {
    let mut ctx = ctx();
    let mut t = loaded(&mut ctx);
    assert_eq!(statuses(&t), ["Running"; 3]);
    t.set_hung(vec![("org.kde.ark".into(), 42)], &mut ctx);
    assert_eq!(statuses(&t), ["Not Responding", "Running", "Running"]);
    t.set_hung(Vec::new(), &mut ctx);
    assert_eq!(
        statuses(&t),
        ["Running"; 3],
        "an answer brings Running back"
    );
}

#[test]
fn applications_lists_only_programs_showing_a_window() {
    let mut ctx = ctx();
    let mut t = mgr();
    t.take(
        Snapshot {
            windows: Some(vec![window("org.kde.ark", 41)]),
            ..snapshot()
        },
        &mut ctx,
    );
    let names: Vec<&str> = t.app_rows.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["Ark"], "a tray or background program is left out");
    t.take(
        Snapshot {
            windows: Some(vec![window("reprise.notepad", 8888)]),
            ..snapshot()
        },
        &mut ctx,
    );
    let rows: Vec<(Option<&str>, Option<u64>)> = (t.app_rows.iter())
        .map(|r| (r.unit.as_deref(), r.window.as_ref().map(|w| w.id)))
        .collect();
    assert_eq!(
        rows,
        [(None, Some(8888))],
        "a host window outside every scope is a row of its own, not Notepad's scope"
    );
}

/// A pid past the kernel's largest, so no process on the machine running the tests has it.
const LONE_PID: u32 = 4_194_400;

/// The user's process `LONE_PID` started at `start`, beside the scoped ones, showing `windows`.
fn lone_snapshot(start: u64, windows: Vec<AppWindow>) -> Snapshot {
    let mut s = snapshot();
    s.procs.push(ProcRow {
        start,
        ..proc(LONE_PID as i32, "calc", UID, 0.0, 1_024)
    });
    s.windows = Some(windows);
    s
}

fn calculator() -> AppWindow {
    titled(7, "reprise.calc", LONE_PID, "Calculator")
}

#[test]
fn applications_lists_a_window_whose_client_runs_in_no_app_scope() {
    let mut ctx = ctx();
    let mut t = mgr();
    let about = AppWindow {
        button: false,
        ..titled(8, "reprise.calc", LONE_PID, "About")
    };
    let nameless = AppWindow {
        app_id: String::new(),
        ..titled(9, "", LONE_PID, "")
    };
    let unknown = AppWindow {
        app_id: String::new(),
        ..titled(10, "", 0, "")
    };
    t.take(
        lone_snapshot(
            1,
            vec![
                titled(1, "org.kde.ark", 41, ""),
                calculator(),
                about,
                nameless,
                unknown,
            ],
        ),
        &mut ctx,
    );
    t.event(MAIN, Event::Resized, &mut ctx);
    assert_eq!(
        tasks(&t),
        ["Ark", "Calculator", "calc", "Untitled window"],
        "by title, else by its process's name, else as untitled; a box has no row"
    );
}

thread_local! {
    static OPENED: std::cell::RefCell<Vec<(i32, u64)>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn record_open(pid: i32, start: u64) -> std::io::Result<OwnedFd> {
    OPENED.with_borrow_mut(|o| o.push((pid, start)));
    Err(std::io::Error::from_raw_os_error(libc::ESRCH))
}

/// End Task on `calculator`, then End Now offered once the wait is up, with the client sampled as
/// started at `first` and then at `then`.
fn end_now_on_calculator(t: &mut TaskMgr, ctx: &mut Ctx, first: u64, then: u64) -> Option<Ask> {
    t.close_window = record_close;
    t.open_process = record_open;
    t.take(lone_snapshot(first, vec![calculator()]), ctx);
    t.event(MAIN, Event::Resized, ctx);
    click_row(t, ctx, 0);
    t.event(MAIN, key(keysym::DELETE), ctx);
    assert_eq!(CLOSED.take(), [7], "its window is asked to close");
    if let Some(e) = t.ending.first_mut() {
        e.since -= END_WAIT;
    }
    t.take(lone_snapshot(then, vec![calculator()]), ctx);
    t.asks.up().map(|(_, a)| a.clone())
}

#[test]
fn end_now_on_a_window_in_no_app_scope_signals_the_client_first_seen_through_a_pidfd() {
    let mut ctx = ctx();
    let mut t = mgr();
    let ask = end_now_on_calculator(&mut t, &mut ctx, 900, 900).expect("End Now after the wait");
    t.asks.close();
    t.answered(ask, Some(0), &ctx);
    assert_eq!(
        OPENED.take(),
        [(LONE_PID as i32, 900)],
        "a pidfd for the pid and start time sampled with the window up"
    );
    t.take(lone_snapshot(900, Vec::new()), &mut ctx);
    assert!(
        t.asks.is_empty() && t.ending.is_empty(),
        "done once it closed"
    );
}

#[test]
fn a_window_whose_client_pid_names_another_process_since_is_not_offered_end_now() {
    let mut ctx = ctx();
    let mut t = mgr();
    t.take(lone_snapshot(900, vec![calculator()]), &mut ctx);
    t.take(lone_snapshot(901, vec![calculator()]), &mut ctx);
    assert!(
        end_now_on_calculator(&mut t, &mut ctx, 901, 901).is_none(),
        "the pid was first seen as a process started at 900"
    );
}

#[test]
fn a_window_whose_client_was_gone_when_first_seen_is_never_offered_end_now() {
    let mut ctx = ctx();
    let mut t = mgr();
    let mut gone = lone_snapshot(900, vec![calculator()]);
    gone.procs.pop();
    t.take(gone, &mut ctx);
    assert!(
        end_now_on_calculator(&mut t, &mut ctx, 900, 900).is_none(),
        "a process given the dead client's pid since is not the window's"
    );
}

#[test]
fn a_window_in_no_app_scope_is_offered_end_now_only_for_the_users_own_process_outside_services() {
    let ask = |pid: u32, uid: u32, service: bool| {
        let mut ctx = ctx();
        let mut t = mgr();
        t.in_service = if service { |_| true } else { |_| false };
        let mut s = lone_snapshot(900, vec![titled(7, "reprise.calc", pid, "Calculator")]);
        s.procs.last_mut().unwrap().uid = uid;
        t.close_window = record_close;
        t.take(s.clone(), &mut ctx);
        t.event(MAIN, Event::Resized, &mut ctx);
        click_row(&mut t, &mut ctx, 0);
        t.event(MAIN, key(keysym::DELETE), &mut ctx);
        CLOSED.take();
        if let Some(e) = t.ending.first_mut() {
            e.since -= END_WAIT;
        }
        t.take(s, &mut ctx);
        t.asks.up().is_some()
    };
    assert!(ask(LONE_PID, UID, false));
    assert!(
        !ask(0, UID, false),
        "no pid: Reprise itself or an unknown client"
    );
    assert!(!ask(LONE_PID, 0, false), "another user's process");
    assert!(!ask(LONE_PID, UID, true), "a portal or other service");
}

fn titled(id: u64, app_id: &str, pid: u32, title: &str) -> AppWindow {
    AppWindow {
        id,
        title: title.into(),
        ..window(app_id, pid)
    }
}

/// Two Notepad windows and its Save As box, and an untitled Ark window.
fn notepads() -> Snapshot {
    let save = AppWindow {
        button: false,
        ..titled(3, "reprise.notepad", 40, "Save As")
    };
    Snapshot {
        windows: Some(vec![
            titled(1, "reprise.notepad", 40, "a.txt - Notepad"),
            titled(2, "reprise.notepad", 40, "b.txt - Notepad"),
            save,
            titled(4, "org.kde.ark", 41, ""),
        ]),
        ..snapshot()
    }
}

fn tasks(t: &TaskMgr) -> Vec<String> {
    t.apps
        .view
        .rows
        .iter()
        .map(|r| r.cells[0].clone())
        .collect()
}

#[test]
fn applications_lists_each_window_with_a_task_button_by_its_title() {
    let mut ctx = ctx();
    let mut t = mgr();
    t.take(notepads(), &mut ctx);
    t.event(MAIN, Event::Resized, &mut ctx);
    assert_eq!(
        tasks(&t),
        ["Ark", "a.txt - Notepad", "b.txt - Notepad"],
        "an untitled window shows its program's name; an owned box has no row"
    );
    click_row(&mut t, &mut ctx, 2);
    let mut s = notepads();
    s.windows.as_mut().unwrap()[1].title = "c.txt - Notepad".into();
    t.take(s, &mut ctx);
    assert_eq!(
        t.selected_app()
            .and_then(|r| r.window.as_ref())
            .map(|w| w.id),
        Some(2),
        "the selection follows its window through a new title"
    );
}

thread_local! {
    static CLOSED: std::cell::RefCell<Vec<u64>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn record_close(w: AppWindow) {
    CLOSED.with_borrow_mut(|c| c.push(w.id));
}

#[test]
fn end_task_on_one_of_a_programs_windows_closes_only_that_one() {
    let mut ctx = ctx();
    let mut t = mgr();
    t.end_task = record_end;
    t.close_window = record_close;
    t.take(notepads(), &mut ctx);
    t.event(MAIN, Event::Resized, &mut ctx);
    click_row(&mut t, &mut ctx, 1);
    t.event(MAIN, key(keysym::DELETE), &mut ctx);
    assert_eq!(CLOSED.take(), [1], "a.txt's window is asked to close");
    assert!(ENDED.take().is_empty(), "Notepad is not ended");
    let unit = t.app_rows[1].unit.clone().unwrap();
    assert!(matches!(&t.ending[..], [e] if e.target == Target::Window(unit.clone(), 1)));
    let mut s = notepads();
    s.windows.as_mut().unwrap().remove(0);
    t.take(s.clone(), &mut ctx);
    assert!(t.ending.is_empty(), "done once that window is gone");
    assert_eq!(tasks(&t), ["Ark", "b.txt - Notepad"]);
    click_row(&mut t, &mut ctx, 1);
    t.event(MAIN, key(keysym::DELETE), &mut ctx);
    assert!(CLOSED.take().is_empty());
    assert_eq!(
        ENDED.take(),
        [(unit, vec![40], false)],
        "the last window ends its program"
    );
}

#[test]
fn a_window_is_its_program_by_pid_and_a_sandboxed_one_by_app_id() {
    let s = snapshot();
    let entries = [
        entry("org.kde.ark", "Ark", None),
        entry("reprise-notepad", "Notepad", Some("reprise.notepad")),
    ];
    let sandboxed = HashSet::from([9999]);
    let shows = |sc: usize, id: &str, pid: u32| {
        has_window(
            &s.scopes[sc],
            &s.scopes,
            &entries,
            &[window(id, pid)],
            &sandboxed,
        )
    };
    assert!(shows(0, "anything", 41), "its client runs in the scope");
    assert!(
        !shows(0, "org.kde.ark", 40),
        "a client in another scope is that program's"
    );
    assert!(
        shows(1, "reprise.notepad", 9999),
        "a sandboxed client in no scope goes by its app id"
    );
    assert!(shows(1, "reprise.notepad", 0), "so does one with no pid");
    assert!(!shows(0, "reprise.notepad", 9999));
    assert!(
        !shows(1, "reprise.notepad", 8888),
        "a host client in no scope is another program's, whatever its app id"
    );
}

thread_local! {
    static ENDED: std::cell::RefCell<Vec<(String, Vec<i32>, bool)>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn record_end(unit: String, pids: Vec<i32>, spare: bool) {
    ENDED.with_borrow_mut(|e| e.push((unit, pids, spare)));
}

#[test]
fn end_task_on_a_program_that_answers_closes_its_windows_and_waits_for_it() {
    let mut ctx = ctx();
    let mut t = loaded(&mut ctx);
    t.end_task = record_end;
    click_row(&mut t, &mut ctx, 0);
    t.event(MAIN, key(keysym::DELETE), &mut ctx);
    let unit = t.app_rows[0].unit.clone().unwrap();
    let pids = (t.snap.scopes.iter().find(|s| s.unit == unit))
        .unwrap()
        .pids
        .clone();
    assert_eq!(
        ENDED.take(),
        [(unit.clone(), pids, false)],
        "the program's processes are asked to close their windows"
    );
    assert!(t.asks.is_empty(), "End Now waits");
    assert!(matches!(&t.ending[..], [e] if e.target == Target::Scope(unit.clone())));
}

#[test]
fn end_task_on_a_program_listed_for_a_sandboxed_window_closes_it_and_spares_the_scope() {
    let mut ctx = ctx();
    let mut t = mgr();
    t.end_task = record_end;
    t.take(
        Snapshot {
            windows: Some(vec![window("org.kde.ark", 9999)]),
            sandboxed: HashSet::from([9999]),
            ..snapshot()
        },
        &mut ctx,
    );
    t.event(MAIN, Event::Resized, &mut ctx);
    let names: Vec<&str> = t.app_rows.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["Ark"]);
    click_row(&mut t, &mut ctx, 0);
    t.event(MAIN, key(keysym::DELETE), &mut ctx);
    assert_eq!(
        ENDED.take(),
        [(
            t.app_rows[0].unit.clone().unwrap(),
            vec![41, 42, 9999],
            true
        )],
        "the sandboxed window is asked to close, and the scope is never signalled"
    );
}

#[test]
fn end_task_is_done_once_the_program_shows_no_window_though_it_keeps_running() {
    let showing = |windows: Vec<AppWindow>| Snapshot {
        windows: Some(windows),
        ..snapshot()
    };
    let mut ctx = ctx();
    let mut t = mgr();
    t.end_task = record_end;
    t.take(showing(vec![window("org.kde.ark", 41)]), &mut ctx);
    t.event(MAIN, Event::Resized, &mut ctx);
    click_row(&mut t, &mut ctx, 0);
    t.event(MAIN, key(keysym::DELETE), &mut ctx);
    ENDED.take();
    t.ending[0].since -= END_WAIT;
    t.take(showing(Vec::new()), &mut ctx);
    assert!(
        t.ending.is_empty(),
        "its window closed, so End Task is done"
    );
    assert!(t.asks.is_empty(), "no End Now for a program left running");

    t.take(showing(vec![window("org.kde.ark", 41)]), &mut ctx);
    click_row(&mut t, &mut ctx, 0);
    t.event(MAIN, key(keysym::DELETE), &mut ctx);
    t.ending[0].since -= END_WAIT;
    t.take(showing(vec![window("org.kde.ark", 41)]), &mut ctx);
    assert!(
        matches!(t.asks.up(), Some((_, Ask::EndNow(Target::Scope(_))))),
        "a window still up after the wait is offered End Now"
    );
    t.take(showing(Vec::new()), &mut ctx);
    assert!(t.asks.is_empty(), "End Now goes once the window does");
}

#[test]
fn end_task_on_a_program_not_responding_offers_end_now_at_once() {
    let mut ctx = ctx();
    let mut t = loaded(&mut ctx);
    t.end_task = record_end;
    t.set_hung(vec![("org.kde.ark".into(), 41)], &mut ctx);
    click_row(&mut t, &mut ctx, 0);
    t.event(MAIN, key(keysym::DELETE), &mut ctx);
    let unit = t.app_rows[0].unit.clone().unwrap();
    assert!(matches!(t.asks.up(), Some((_, Ask::EndNow(Target::Scope(u)))) if *u == unit));
    assert!(
        t.ending.is_empty(),
        "nothing waits on a request it cannot answer"
    );
    assert!(ENDED.take().is_empty(), "no window is asked to close");
}

#[test]
fn always_on_top_is_ticked_kept_and_follows_the_window_menu() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("taskmgr.toml");
    let mut ctx = ctx();
    let mut t = loaded(&mut ctx);
    t.restore(path.clone());
    let ticked = |t: &TaskMgr| {
        matches!(
            t.bar.menus[1].items()[0],
            MenuItem::Entry { checked: true, .. }
        )
    };
    assert_eq!(t.on_top(), Some(false));
    assert!(!ticked(&t));
    let act = t.menu_act(1, &[0]).expect("Options > Always On Top");
    t.act(act);
    assert!(ticked(&t) && t.on_top() == Some(true));
    let mut again = loaded(&mut ctx);
    again.restore(path.clone());
    assert_eq!(again.on_top(), Some(true), "it survives a restart");
    again.event(MAIN, Event::OnTop(false), &mut ctx);
    assert!(!ticked(&again), "the window menu unticks it");
    let mut third = loaded(&mut ctx);
    third.restore(path);
    assert_eq!(third.on_top(), Some(false), "and that is kept too");
}

#[test]
fn the_histories_take_each_sample() {
    let mut ctx = ctx();
    let t = loaded(&mut ctx);
    assert_eq!(t.cpu.latest(), Some(0.25));
    assert_eq!(t.cores.len(), 2);
    assert_eq!(t.mem.latest(), Some(0.25));
    assert_eq!(t.swap.latest(), Some(0.0));
}

#[test]
fn sizes_are_grouped_by_thousands() {
    assert_eq!(kb_text(40_960), "40,960 K");
    assert_eq!(grouped(1_234_567), "1,234,567");
    assert_eq!(grouped(999), "999");
}

#[test]
fn a_process_list_that_scrolls_has_no_horizontal_bar() {
    let mut ctx = ctx();
    let mut t = mgr();
    let mut snap = snapshot();
    snap.procs = (100..160)
        .map(|pid| proc(pid, "busy", UID, 0.0, 1_024))
        .collect();
    t.event(MAIN, Event::Resized, &mut ctx);
    t.take(snap, &mut ctx);
    open(&mut t, &mut ctx, Page::Processes);
    let r = pages::layout(&t, &ctx).list_area;
    let lay = t.procs.layout(r, &ctx);
    assert!(
        lay.scrollbar.is_some(),
        "sixty rows that came after the window sized scroll"
    );
    assert!(lay.hscroll.is_none(), "the columns fit beside the bar");
}

#[test]
fn per_core_graphs_wrap_past_eight() {
    let r = reprisekit::raster::Rect::new(0, 0, 200, 100);
    assert_eq!(perf::core_rects(r, 2).len(), 2);
    let twelve = perf::core_rects(r, 12);
    assert_eq!(twelve[8].y, twelve[0].y + twelve[0].height + 3);
    assert!(perf::core_rects(r, 0).is_empty());
}

pub(crate) fn fnv(c: &Canvas) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for v in [c.width, c.height]
        .into_iter()
        .chain(c.pixels.iter().copied())
    {
        for b in v.to_le_bytes() {
            h = (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3);
        }
    }
    h
}

/// [`reprisekit::pins::check_file`] on `tests/pins/<name>.txt`.
#[track_caller]
pub(crate) fn pins(name: &str, got: &[(String, u64)]) {
    let path = reprisekit::crate_dir!()
        .join("tests/pins")
        .join(format!("{name}.txt"));
    reprisekit::pins::check_file(&path, got);
}

#[test]
fn the_pages_are_pinned_in_every_interface_at_1_and_2() {
    reprisekit::pins::fonts();
    let mut got = Vec::new();
    for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
        let mut ctx = themed_ctx(reprisekit::interfaces::theme_for(i));
        let mut t = loaded(&mut ctx);
        for v in [0.1, 0.6, 0.3, 0.9, 0.4] {
            t.cpu.push(v);
            t.mem.push(v / 2.0);
        }
        for page in Page::ALL {
            open(&mut t, &mut ctx, page);
            if page == Page::Processes {
                click_row(&mut t, &mut ctx, 1);
            }
            if page == Page::Services {
                list_services(&mut t, &mut ctx, service_rows(), true);
                click_row(&mut t, &mut ctx, 0);
            }
            t.event(MAIN, Event::Pointer(PointerInput::Leave), &mut ctx);
            for d in [1, 2] {
                t.follow_scale(MAIN, d, &mut ctx);
                let (w, h) = ctx.size;
                let mut c = Canvas::with_scale(w as u32, h as u32, d);
                ctx.at_scale(d, |ctx| t.draw(MAIN, &mut c, ctx));
                got.push((format!("{i:?} {page:?} @{d}"), fnv(&c)));
            }
        }
        open(&mut t, &mut ctx, Page::Services);
        let mut rows = service_rows();
        rows[1].status = services::Status::Failed;
        list_services(&mut t, &mut ctx, rows, true);
        t.event(MAIN, alt('a'), &mut ctx);
        t.event(MAIN, alt('i'), &mut ctx);
        for ch in "sy".chars() {
            t.event(MAIN, typed(ch), &mut ctx);
        }
        t.event(MAIN, Event::Pointer(PointerInput::Leave), &mut ctx);
        for d in [1, 2] {
            t.follow_scale(MAIN, d, &mut ctx);
            let (w, h) = ctx.size;
            let mut c = Canvas::with_scale(w as u32, h as u32, d);
            ctx.at_scale(d, |ctx| t.draw(MAIN, &mut c, ctx));
            got.push((format!("{i:?} Services filtered @{d}"), fnv(&c)));
        }
        t.event(MAIN, alt('a'), &mut ctx);
        t.event(MAIN, alt('i'), &mut ctx);
        for _ in 0..2 {
            t.event(MAIN, backspace(), &mut ctx);
        }
        t.event(MAIN, key(keysym::TAB), &mut ctx);
        list_services(&mut t, &mut ctx, service_rows(), true);
        open(&mut t, &mut ctx, Page::Applications);
        t.set_hung(vec![("org.kde.ark".into(), 41)], &mut ctx);
        t.event(MAIN, Event::Pointer(PointerInput::Leave), &mut ctx);
        for d in [1, 2] {
            t.follow_scale(MAIN, d, &mut ctx);
            let (w, h) = ctx.size;
            let mut c = Canvas::with_scale(w as u32, h as u32, d);
            ctx.at_scale(d, |ctx| t.draw(MAIN, &mut c, ctx));
            got.push((format!("{i:?} Applications not responding @{d}"), fnv(&c)));
        }
        t.set_hung(Vec::new(), &mut ctx);
        t.take(notepads(), &mut ctx);
        click_row(&mut t, &mut ctx, 2);
        t.event(MAIN, Event::Pointer(PointerInput::Leave), &mut ctx);
        for d in [1, 2] {
            t.follow_scale(MAIN, d, &mut ctx);
            let (w, h) = ctx.size;
            let mut c = Canvas::with_scale(w as u32, h as u32, d);
            ctx.at_scale(d, |ctx| t.draw(MAIN, &mut c, ctx));
            got.push((format!("{i:?} Applications windows @{d}"), fnv(&c)));
        }
        t.exe_of = ark_file;
        open(&mut t, &mut ctx, Page::Processes);
        click_row(&mut t, &mut ctx, 0);
        t.event(MAIN, Event::Pointer(PointerInput::Leave), &mut ctx);
        for d in [1, 2] {
            t.follow_scale(MAIN, d, &mut ctx);
            t.event(MAIN, key(keysym::MENU), &mut ctx);
            let (w, h) = ctx.size;
            let mut c = Canvas::with_scale(w as u32, h as u32, d);
            ctx.at_scale(d, |ctx| t.draw(MAIN, &mut c, ctx));
            got.push((format!("{i:?} Processes menu @{d}"), fnv(&c)));
            t.event(MAIN, key(keysym::ESCAPE), &mut ctx);
        }
        t.act(Act::SelectColumns);
        let main = ctx.size;
        ctx.size = ctx.forms().text.size(selectcols::CLIENT);
        for d in [1, 2] {
            let (w, h) = ctx.size;
            let mut c = Canvas::with_scale(w as u32, h as u32, d);
            ctx.at_scale(d, |ctx| t.draw(PICK, &mut c, ctx));
            got.push((format!("{i:?} Select Columns @{d}"), fnv(&c)));
        }
        ctx.size = main;
        t.pick = None;
        open(&mut t, &mut ctx, Page::Services);
        click_row(&mut t, &mut ctx, 2);
        t.event(MAIN, Event::Pointer(PointerInput::Leave), &mut ctx);
        for d in [1, 2] {
            t.follow_scale(MAIN, d, &mut ctx);
            t.event(MAIN, key(keysym::MENU), &mut ctx);
            let (w, h) = ctx.size;
            let mut c = Canvas::with_scale(w as u32, h as u32, d);
            ctx.at_scale(d, |ctx| t.draw(MAIN, &mut c, ctx));
            got.push((format!("{i:?} Services menu @{d}"), fnv(&c)));
            t.event(MAIN, key(keysym::ESCAPE), &mut ctx);
        }
        t.svc.read_log = |_, _, _| {};
        t.open_log(&ctx);
        let log = t.svc.log.as_mut().unwrap();
        // An entry's time follows the local time zone, which the pin environment leaves alone.
        log.show(Err(services::LogError::Exited(
            1,
            "No journal files were found.".into(),
        )));
        ctx.size = ctx.forms().text.size(servicelog::CLIENT);
        for d in [1, 2] {
            let (w, h) = ctx.size;
            let mut c = Canvas::with_scale(w as u32, h as u32, d);
            ctx.at_scale(d, |ctx| t.draw(LOG, &mut c, ctx));
            got.push((format!("{i:?} Service log @{d}"), fnv(&c)));
        }
        ctx.size = main;
    }
    pins("pages", &got);
}

#[test]
#[ignore]
fn render_pages() {
    for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
        let mut ctx = themed_ctx(reprisekit::interfaces::theme_for(i));
        let mut t = loaded(&mut ctx);
        t.take(notepads(), &mut ctx);
        for k in 0..60 {
            t.cpu.push(0.4 + (f64::from(k) * 0.4).sin() * 0.3);
            t.mem.push(0.3);
            t.swap.push(0.05);
        }
        for page in Page::ALL {
            open(&mut t, &mut ctx, page);
            if page == Page::Services {
                list_services(&mut t, &mut ctx, service_rows(), true);
            }
            let (w, h) = ctx.size;
            for d in [1, 2] {
                let mut c = Canvas::with_scale(w as u32, h as u32, d);
                ctx.at_scale(d, |ctx| t.draw(MAIN, &mut c, ctx));
                reprisekit::raster::write_preview(&c, &format!("taskmgr-{i:?}-{page:?}@{d}.png"));
            }
        }
        open(&mut t, &mut ctx, Page::Services);
        t.event(MAIN, alt('i'), &mut ctx);
        for ch in "sy".chars() {
            t.event(MAIN, typed(ch), &mut ctx);
        }
        let (w, h) = ctx.size;
        for d in [1, 2] {
            let mut c = Canvas::with_scale(w as u32, h as u32, d);
            ctx.at_scale(d, |ctx| t.draw(MAIN, &mut c, ctx));
            reprisekit::raster::write_preview(&c, &format!("taskmgr-{i:?}-filtered@{d}.png"));
        }
        t.act(Act::SelectColumns);
        ctx.size = ctx.forms().text.size(selectcols::CLIENT);
        let (w, h) = ctx.size;
        for d in [1, 2] {
            let mut c = Canvas::with_scale(w as u32, h as u32, d);
            ctx.at_scale(d, |ctx| t.draw(PICK, &mut c, ctx));
            reprisekit::raster::write_preview(&c, &format!("taskmgr-{i:?}-columns@{d}.png"));
        }
    }
}

fn right_click(t: &mut TaskMgr, ctx: &mut Ctx, (x, y): (i32, i32)) {
    t.event(
        MAIN,
        Event::Pointer(PointerInput::Press {
            x,
            y,
            button: repriseapp::BTN_RIGHT,
            time: 0,
            mods: Mods::default(),
        }),
        ctx,
    );
}

fn view_marks(t: &TaskMgr) -> Vec<bool> {
    let view = t.bar.menus[2].at(&[3]).unwrap();
    (view.items().iter())
        .map(|i| matches!(i, MenuItem::Entry { checked: true, .. }))
        .collect()
}

#[test]
fn the_cpu_graph_menu_picks_the_same_view_as_the_view_menu() {
    let mut ctx = ctx();
    let mut t = loaded(&mut ctx);
    open(&mut t, &mut ctx, Page::Performance);
    let lay = pages::layout(&t, &ctx).perf.unwrap();
    right_click(&mut t, &mut ctx, centre(perf::cpu_graph(&lay).unwrap()));
    assert!(t.popup.is_some());
    assert!(
        t.popup(MAIN, &ctx).is_some(),
        "the menu is hosted on a popup"
    );
    for k in [keysym::DOWN, keysym::DOWN, keysym::RETURN] {
        t.event(MAIN, key(k), &mut ctx);
    }
    assert!(t.popup.is_none());
    assert!(t.per_core, "One Graph Per CPU");
    assert_eq!(view_marks(&t), [false, true], "View > CPU History follows");

    t.event(MAIN, key(keysym::MENU), &mut ctx);
    assert!(
        t.popup.is_some(),
        "the Menu key opens it on the Performance page"
    );
    t.event(MAIN, key(keysym::RETURN), &mut ctx);
    assert!(!t.per_core, "by keyboard its first row is picked");
    assert_eq!(view_marks(&t), [true, false]);

    t.perf.list.select_row(Some(1));
    t.event(MAIN, key(keysym::MENU), &mut ctx);
    assert!(t.popup.is_none(), "no menu while another pane shows");
    let mem = pages::layout(&t, &ctx).perf.unwrap();
    right_click(&mut t, &mut ctx, centre(mem.rows[0].graphs[0].group));
    assert!(t.popup.is_none(), "the memory graph has no menu");
}

#[test]
fn shift_f10_opens_the_graph_menu_and_f10_still_opens_the_menu_bar() {
    let mut ctx = ctx();
    let mut t = loaded(&mut ctx);
    open(&mut t, &mut ctx, Page::Performance);
    let shift_f10 = Event::Key(KeyInput {
        sym: keysym::F10,
        utf8: None,
        mods: Mods {
            shift: true,
            ..Mods::default()
        },
        edit: None,
    });
    t.event(MAIN, shift_f10, &mut ctx);
    assert!(t.popup.is_some());
    assert!(t.bar.active.is_none());
    t.event(MAIN, key(keysym::ESCAPE), &mut ctx);
    assert!(t.popup.is_none());
    t.event(MAIN, key(keysym::F10), &mut ctx);
    assert!(t.bar.active.is_some());
    t.event(MAIN, key(keysym::ESCAPE), &mut ctx);
    open(&mut t, &mut ctx, Page::Processes);
    t.event(MAIN, key(keysym::MENU), &mut ctx);
    assert!(
        t.popup.is_none(),
        "on Processes the menu is a selected process's"
    );
}

thread_local! {
    static RUNS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[test]
fn new_task_opens_the_run_box_from_the_file_menu_and_the_applications_page() {
    let mut ctx = ctx();
    let mut t = loaded(&mut ctx);
    t.new_task = || RUNS.set(RUNS.get() + 1);
    let act = t.menu_act(0, &[0]).expect("File > New Task (Run...)");
    t.act(act);
    pages::follow_form(&mut t, &ctx);
    assert!(enabled(&t, pages::NEW_TASK), "with nothing selected too");
    let b = t.form.rect(pages::NEW_TASK).unwrap();
    click(&mut t, &mut ctx, centre(b));
    assert_eq!(RUNS.get(), 2);
    assert_eq!(t.menu_act(0, &[2]), Some(Act::Quit), "Exit stays last");
}

/// Ark (pid 41) runs `/usr/bin/ark`; the other processes' files cannot be read.
fn ark_file(pid: i32) -> Option<PathBuf> {
    (pid == 41).then(|| PathBuf::from("/usr/bin/ark"))
}

/// What the selected process's menu rows do, None for a greyed one.
fn live_rows(t: &TaskMgr) -> Vec<Option<Act>> {
    (t.proc_menu().expect("a menu").into_iter())
        .filter(|(i, _)| !matches!(i, MenuItem::Separator))
        .map(|(_, a)| a)
        .collect()
}

#[test]
fn a_process_menu_opens_its_files_folder_or_its_properties_and_ends_it() {
    let mut ctx = ctx();
    let mut t = loaded(&mut ctx);
    t.exe_of = ark_file;
    t.open_process = |_, _| reprisebus::pidfd::open(std::process::id());
    open(&mut t, &mut ctx, Page::Processes);
    assert_eq!(t.proc_rows[0].pid, 41);
    let at = row_point(&t, &ctx, 0);
    right_click(&mut t, &mut ctx, at);
    assert_eq!(
        t.selected_proc().map(|p| p.pid),
        Some(41),
        "the click selects"
    );
    assert!(t.popup.is_some());
    assert_eq!(
        live_rows(&t),
        [
            Some(Act::OpenLocation),
            Some(Act::EndProcess),
            Some(Act::Properties)
        ]
    );
    let r = t.event(MAIN, key(keysym::RETURN), &mut ctx);
    assert!(
        r.spawn.is_empty(),
        "a pointer-opened menu has no row picked"
    );
    t.popup = None;
    t.event(MAIN, key(keysym::MENU), &mut ctx);
    let r = t.event(MAIN, key(keysym::RETURN), &mut ctx);
    assert_eq!(
        r.spawn,
        [reprisefs::apps::open_location_argv("/usr/bin/ark".as_ref())],
        "Open File Location is the first row"
    );
    assert_eq!(
        t.act(Act::Properties).spawn,
        [reprisefs::apps::properties_argv("/usr/bin/ark".as_ref())]
    );
    t.event(MAIN, key(keysym::MENU), &mut ctx);
    for k in [keysym::DOWN, keysym::RETURN] {
        t.event(MAIN, key(k), &mut ctx);
    }
    assert!(
        matches!(t.asks.up(), Some((_, Ask::EndProcess { pid: 41, .. }))),
        "End Process asks as the button does"
    );
    t.asks.close();
    click_row(&mut t, &mut ctx, 1);
    assert_eq!(
        live_rows(&t),
        [None, Some(Act::EndProcess), None],
        "a process whose file cannot be read has only End Process"
    );
}

/// Task Manager on the Processes page with more rows than fit, the point on its thumb and the
/// scrollbar's rect.
fn overflowing_processes(ctx: &mut Ctx) -> (TaskMgr, (i32, i32), reprisekit::raster::Rect) {
    use reprisekit::classic::widgets::scrollbar_parts;
    let mut t = mgr();
    let mut s = snapshot();
    s.procs
        .extend((100..160).map(|pid| proc(pid, "worker", UID, 0.0, 1_024)));
    t.take(s, ctx);
    t.event(MAIN, Event::Resized, ctx);
    open(&mut t, ctx, Page::Processes);
    let r = pages::layout(&t, ctx).list.unwrap();
    let lay = t.procs.layout(r, ctx);
    let sb = lay.scrollbar.expect("the processes overflow the list");
    let p = scrollbar_parts(
        sb,
        1,
        0,
        lay.rows,
        t.procs.view.slots(),
        false,
        lay.thumb_min,
    );
    let thumb = p.thumb.expect("a thumb");
    (t, (thumb.x + 2, thumb.y + 3), sb)
}

fn press_at(t: &mut TaskMgr, ctx: &mut Ctx, (x, y): (i32, i32)) {
    let press = PointerInput::Press {
        x,
        y,
        button: BTN_LEFT,
        time: 0,
        mods: Mods::default(),
    };
    t.event(MAIN, Event::Pointer(press), ctx);
}

#[test]
fn a_covered_window_lets_go_of_a_held_thumb_and_divider() {
    let mut ctx = ctx();
    let (mut t, (x, y), sb) = overflowing_processes(&mut ctx);
    press_at(&mut t, &mut ctx, (x, y));
    repriseapp::cover(&mut t, MAIN, &ctx);
    let motion = PointerInput::Motion { x, y: sb.bottom() };
    t.event(MAIN, Event::Pointer(motion), &mut ctx);
    assert_eq!(t.procs.view.top, 0, "the list stays at its top");
    let r = pages::layout(&t, &ctx).list.unwrap();
    let l = t.procs.layout(r, &ctx);
    let (edge, y) = (edges(&t, &l)[1], l.header.y + 4);
    press_at(&mut t, &mut ctx, (edge, y));
    let motion = |x| Event::Pointer(PointerInput::Motion { x, y });
    t.event(MAIN, motion(edge + 30), &mut ctx);
    repriseapp::cover(&mut t, MAIN, &ctx);
    t.event(MAIN, motion(edge + 60), &mut ctx);
    assert_eq!(t.procs.view.columns[1].width, 78, "PID stays 78 wide");
    assert_eq!(t.widths.get(&ProcColumn::Pid), Some(&78), "78 is kept");
}

#[test]
fn a_thumb_held_across_a_page_switch_lets_go_at_the_release() {
    let mut ctx = ctx();
    let (mut t, (x, y), sb) = overflowing_processes(&mut ctx);
    let ctrl_tab = |shift| {
        Event::Key(KeyInput {
            sym: keysym::TAB,
            utf8: None,
            mods: Mods {
                ctrl: true,
                shift,
                alt: false,
            },
            edit: None,
        })
    };
    let press = PointerInput::Press {
        x,
        y,
        button: BTN_LEFT,
        time: 0,
        mods: Mods::default(),
    };
    t.event(MAIN, Event::Pointer(press), &mut ctx);
    t.event(MAIN, ctrl_tab(false), &mut ctx);
    assert_ne!(t.page(), Page::Processes);
    click_release(&mut t, &mut ctx, (x, y));
    t.event(MAIN, ctrl_tab(true), &mut ctx);
    assert_eq!(t.page(), Page::Processes);
    let motion = PointerInput::Motion { x, y: sb.bottom() };
    t.event(MAIN, Event::Pointer(motion), &mut ctx);
    assert_eq!(
        t.procs.view.top, 0,
        "a motion with no button down drags nothing"
    );
}

#[test]
fn at_200_percent_text_the_page_buttons_and_the_performance_heading_land_inside_the_page() {
    let mut ctx = themed_ctx(Theme::default().with_text_size(200));
    ctx.size = ctx.theme.text.size(pages::CLIENT);
    let mut t = mgr();
    let mut c = Canvas::new(ctx.size.0 as u32, ctx.size.1 as u32);
    for page in [Page::Applications, Page::Performance] {
        t.tabs.open = page as usize;
        pages::draw(&mut t, &mut c, &ctx);
        let lay = pages::layout(&t, &ctx);
        let ids: &[&str] = match page {
            Page::Applications => &[pages::END_TASK, pages::SWITCH_TO],
            _ => &["heading"],
        };
        for id in ids {
            let r = t
                .form
                .rect(id)
                .unwrap_or_else(|| panic!("{id} is on the form"));
            assert!(
                lay.inner.contains(r.x, r.y)
                    && r.right() <= lay.inner.right()
                    && r.bottom() <= lay.inner.bottom(),
                "{page:?} {id} at {r:?} is inside {:?}",
                lay.inner
            );
        }
    }
}

#[test]
fn the_process_tip_comes_up_under_the_pointer_at_scale_2() {
    use reprisekit::controls::tooltip::INFO_BELOW_POINTER;
    let mut ctx = themed_ctx(reprisekit::interfaces::theme_for(Interface::Glass));
    let mut t = loaded(&mut ctx);
    open(&mut t, &mut ctx, Page::Processes);
    let at = row_point(&t, &ctx, 1);
    t.event(
        MAIN,
        Event::Pointer(PointerInput::Motion { x: at.0, y: at.1 }),
        &mut ctx,
    );
    t.tip.as_mut().expect("a tip rests on the row").shown = true;
    t.follow_scale(MAIN, 2, &mut ctx);
    let (w, h) = ctx.size;
    let mut c = Canvas::with_scale(w as u32, h as u32, 2);
    ctx.at_scale(2, |ctx| t.draw(MAIN, &mut c, ctx));
    let bare = c.clone();
    let r = form::draw_tip(&mut c, &ctx.text, &ctx.forms()).expect("the tip is reported");
    assert_eq!(
        (r.x, r.y),
        (at.0 - 1, at.1 + INFO_BELOW_POINTER),
        "the tip's corner sits under the pointer in logical px"
    );
    let drawn = bare.differing_rect(&c);
    assert_eq!(
        (drawn.x, drawn.y),
        (r.x, r.y),
        "the tip's pixels start at its corner"
    );
    assert!(
        drawn.width < r.width + 10 && drawn.height < r.height + 10,
        "the tip is drawn at its logical size plus its shadow, got {drawn:?} for {r:?}"
    );
}

#[test]
fn the_process_tip_closes_when_another_process_sorts_under_the_pointer() {
    let mut ctx = ctx();
    let mut t = loaded(&mut ctx);
    open(&mut t, &mut ctx, Page::Processes);
    let at = row_point(&t, &ctx, 0);
    t.event(
        MAIN,
        Event::Pointer(PointerInput::Motion { x: at.0, y: at.1 }),
        &mut ctx,
    );
    t.tip.as_mut().expect("a tip rests on ark's row").shown = true;
    let tip = |t: &mut TaskMgr, ctx: &mut Ctx| {
        let (w, h) = ctx.size;
        let mut c = Canvas::new(w as u32, h as u32);
        t.draw(MAIN, &mut c, ctx);
        c.take_tip().map(|tip| tip.text)
    };
    t.take(snapshot(), &mut ctx);
    assert_eq!(
        tip(&mut t, &mut ctx).as_deref(),
        Some("/usr/bin/ark --flag"),
        "ark still under the pointer keeps its tip"
    );
    let mut s = snapshot();
    s.procs[1].cpu = 20.0;
    t.take(s, &mut ctx);
    assert_eq!(t.proc_rows[0].name, "notepad", "notepad sorts first");
    assert_eq!(tip(&mut t, &mut ctx), None, "ark's tip closes");
    assert!(t.tip.is_none(), "and does not come back over notepad");
}

#[test]
fn box_texts_keep_their_paragraph_breaks() {
    assert_eq!(
        tr!("end-process-text", name = "foo", pid = "42"),
        "Ending foo (PID 42) can lose the data it holds, and a program that needs it may stop \
         working.\n\nAre you sure you want to end this process?"
    );
    assert_eq!(
        tr!("end-now-asked", name = "foo"),
        "foo is not responding to the request to end.\n\nTo end it now, click End Now. Anything \
         it has not saved is lost."
    );
    assert_eq!(tr!("sensors-heading", count = 1), "1 sensor chip");
    assert_eq!(tr!("sensors-heading", count = 3), "3 sensor chips");
}

fn service(
    name: &str,
    manager: Manager,
    status: services::Status,
    startup: services::Startup,
) -> services::Service {
    services::Service {
        name: name.into(),
        description: format!("{name} described"),
        manager,
        status,
        startup,
    }
}

/// A user service running and enabled, a stopped user service that installs, and a stopped
/// system service.
fn service_rows() -> Vec<services::Service> {
    use services::{Startup, Status};
    vec![
        service(
            "pipewire.service",
            Manager::User,
            Status::Running,
            Startup::Automatic,
        ),
        service(
            "sync.service",
            Manager::User,
            Status::Stopped,
            Startup::Manual,
        ),
        service(
            "sshd.service",
            Manager::System,
            Status::Stopped,
            Startup::Manual,
        ),
    ]
}

/// Posts `rows` as the reader thread does and wakes the window.
fn list_services(t: &mut TaskMgr, ctx: &mut Ctx, rows: Vec<services::Service>, manage: bool) {
    t.svc
        .tx
        .send(services::Msg::Listed {
            rows,
            system: t.svc.system,
            manage_system: manage,
        })
        .unwrap();
    t.event(MAIN, Event::Wake, ctx);
}

thread_local! {
    static JOBS: std::cell::RefCell<Vec<services::Job>> = const { std::cell::RefCell::new(Vec::new()) };
    static LOGS: std::cell::RefCell<Vec<(Manager, String)>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn services_mgr(ctx: &mut Ctx) -> TaskMgr {
    let mut t = loaded(ctx);
    t.svc.run_job = |job, _| JOBS.with_borrow_mut(|j| j.push(job));
    t.svc.read_log = |m, name, _| LOGS.with_borrow_mut(|l| l.push((m, name)));
    open(&mut t, ctx, Page::Services);
    list_services(&mut t, ctx, service_rows(), true);
    t
}

fn job(manager: Manager, name: &str, verb: Verb) -> services::Job {
    services::Job {
        manager,
        name: name.into(),
        verb,
    }
}

fn service_menu_rows(t: &TaskMgr) -> Vec<Option<Act>> {
    (t.service_menu().expect("a menu").into_iter())
        .filter(|(i, _)| !matches!(i, MenuItem::Separator))
        .map(|(_, a)| a)
        .collect()
}

#[test]
fn the_services_page_shows_each_service_by_name_with_its_status_and_startup() {
    let mut ctx = ctx();
    let t = services_mgr(&mut ctx);
    let cells: Vec<Vec<String>> = (t.svc.list.view.rows.iter())
        .map(|r| r.cells.clone())
        .collect();
    assert_eq!(
        cells[0],
        [
            "pipewire",
            "pipewire.service described",
            "Running",
            "Automatic",
            "User"
        ]
    );
    assert_eq!(
        cells.iter().map(|c| c[0].as_str()).collect::<Vec<_>>(),
        ["pipewire", "sshd", "sync"],
        "sorted by name"
    );
    assert_eq!(cells[1][4], "System");
}

#[test]
fn a_column_title_sorts_the_services_and_the_selection_stays() {
    let mut ctx = ctx();
    let mut t = services_mgr(&mut ctx);
    click_row(&mut t, &mut ctx, 2);
    assert_eq!(t.selected_service().unwrap().name, "sync.service");
    t.sort_services(2);
    let names: Vec<&str> = t.svc.rows.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(
        names,
        ["pipewire.service", "sshd.service", "sync.service"],
        "Running before Stopped, then by name"
    );
    t.sort_services(2);
    let names: Vec<&str> = t.svc.rows.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(
        names,
        ["sshd.service", "sync.service", "pipewire.service"],
        "a second click turns the column; a tie stays by name"
    );
    assert_eq!(t.selected_service().unwrap().name, "sync.service");
}

#[test]
fn start_and_restart_run_at_once_and_stop_asks_first() {
    let mut ctx = ctx();
    let mut t = services_mgr(&mut ctx);
    click_row(&mut t, &mut ctx, 2);
    assert!(enabled(&t, pages::START));
    assert!(!enabled(&t, pages::STOP));
    t.form_event(form::Event::Pressed(pages::START), &mut ctx);
    assert_eq!(
        JOBS.take(),
        [job(Manager::User, "sync.service", Verb::Start)]
    );
    pages::follow_form(&mut t, &ctx);
    assert!(
        !enabled(&t, pages::START),
        "a service a job is under way on takes no other"
    );
    t.svc
        .tx
        .send(services::Msg::Done(
            job(Manager::User, "sync.service", Verb::Start),
            Ok(()),
        ))
        .unwrap();
    t.event(MAIN, Event::Wake, &mut ctx);
    assert!(t.svc.busy.is_empty());

    click_row(&mut t, &mut ctx, 0);
    t.form_event(form::Event::Pressed(pages::RESTART), &mut ctx);
    assert_eq!(
        JOBS.take(),
        [job(Manager::User, "pipewire.service", Verb::Restart)]
    );
    t.svc.busy.clear();
    t.form_event(form::Event::Pressed(pages::STOP), &mut ctx);
    assert!(JOBS.take().is_empty(), "Stop waits for the answer");
    let Some((_, Ask::StopService(asked))) = t.asks.up() else {
        panic!("Stop asks first");
    };
    assert_eq!(*asked, job(Manager::User, "pipewire.service", Verb::Stop));
    let asked = asked.clone();
    t.asks.close();
    t.answered(Ask::StopService(asked.clone()), Some(1), &ctx);
    assert!(JOBS.take().is_empty(), "No stops nothing");
    t.answered(Ask::StopService(asked.clone()), Some(0), &ctx);
    assert_eq!(JOBS.take(), [asked]);
}

#[test]
fn a_system_service_takes_no_job_polkit_refuses() {
    let mut ctx = ctx();
    let mut t = services_mgr(&mut ctx);
    click_row(&mut t, &mut ctx, 1);
    assert_eq!(t.selected_service().unwrap().manager, Manager::System);
    pages::follow_form(&mut t, &ctx);
    assert!(enabled(&t, pages::START));
    assert_eq!(
        service_menu_rows(&t),
        [
            Some(Act::Service(Verb::Start)),
            None,
            None,
            Some(Act::OpenLog)
        ],
        "no Start at Login for a system service"
    );
    list_services(&mut t, &mut ctx, service_rows(), false);
    pages::follow_form(&mut t, &ctx);
    assert!(!enabled(&t, pages::START));
    t.form_event(form::Event::Pressed(pages::START), &mut ctx);
    assert!(JOBS.take().is_empty());
}

#[test]
fn start_at_login_enables_or_disables_a_user_service() {
    let mut ctx = ctx();
    let mut t = services_mgr(&mut ctx);
    click_row(&mut t, &mut ctx, 0);
    let menu = t.service_menu().unwrap();
    let login = (menu.iter())
        .find(|(i, _)| matches!(i, MenuItem::Entry { label, .. } if label.contains("Login")))
        .unwrap();
    assert!(matches!(login.0, MenuItem::Entry { checked: true, .. }));
    assert_eq!(login.1, Some(Act::Service(Verb::Disable)));
    click_row(&mut t, &mut ctx, 2);
    assert_eq!(
        service_menu_rows(&t),
        [
            Some(Act::Service(Verb::Start)),
            None,
            None,
            Some(Act::Service(Verb::Enable)),
            Some(Act::OpenLog)
        ]
    );
    t.event(MAIN, key(keysym::MENU), &mut ctx);
    for k in [keysym::DOWN, keysym::RETURN] {
        t.event(MAIN, key(k), &mut ctx);
    }
    assert_eq!(
        JOBS.take(),
        [job(Manager::User, "sync.service", Verb::Enable)]
    );
}

#[test]
fn a_failed_job_says_why_and_a_dismissed_prompt_says_nothing() {
    let mut ctx = ctx();
    let mut t = services_mgr(&mut ctx);
    let done = |r| services::Msg::Done(job(Manager::System, "sshd.service", Verb::Start), r);
    t.svc.tx.send(done(Err(Failed::Dismissed))).unwrap();
    t.event(MAIN, Event::Wake, &mut ctx);
    assert!(!t.errors.is_shown(), "a dismissed prompt is no failure");
    let why = reprisebus::polkit::Told::Text("Access denied".into());
    t.svc.tx.send(done(Err(Failed::Told(why)))).unwrap();
    t.event(MAIN, Event::Wake, &mut ctx);
    assert_eq!(
        t.errors.shown().map(|b| b.fields.text.as_str()),
        Some("sshd could not be started.\n\nAccess denied")
    );
}

#[test]
fn turning_off_start_at_login_for_a_service_every_user_starts_says_why() {
    let mut ctx = ctx();
    let mut t = services_mgr(&mut ctx);
    let off = job(Manager::User, "pipewire.service", Verb::Disable);
    t.svc
        .tx
        .send(services::Msg::Done(off, Err(Failed::Unchanged)))
        .unwrap();
    t.event(MAIN, Event::Wake, &mut ctx);
    let text = t.errors.shown().map(|b| b.fields.text.clone()).unwrap();
    assert!(
        text.starts_with("pipewire could not be kept from starting at login.")
            && text.contains("every user"),
        "{text}"
    );
    click_row(&mut t, &mut ctx, 0);
    assert!(
        (t.service_menu().unwrap().iter()).any(|(i, _)| matches!(
            i,
            MenuItem::Entry { label, checked: true, .. } if label.contains("Login")
        )),
        "the row still reads Automatic, so the check stays on"
    );
}

#[test]
fn a_list_read_before_the_box_changed_is_dropped() {
    let mut ctx = ctx();
    let mut t = services_mgr(&mut ctx);
    t.set_system_services(false);
    t.svc
        .tx
        .send(services::Msg::Listed {
            rows: service_rows(),
            system: true,
            manage_system: true,
        })
        .unwrap();
    t.event(MAIN, Event::Wake, &mut ctx);
    assert!(
        (t.svc.rows.iter()).all(|s| s.manager == Manager::User),
        "the hidden system rows stay hidden"
    );
}

#[test]
fn open_log_shows_the_journal_lines_of_the_one_service() {
    let mut ctx = ctx();
    let mut t = services_mgr(&mut ctx);
    click_row(&mut t, &mut ctx, 0);
    t.event(MAIN, key(keysym::RETURN), &mut ctx);
    assert_eq!(
        LOGS.take(),
        [(Manager::User, "pipewire.service".to_string())]
    );
    let log = t.svc.log.as_ref().expect("the log window");
    assert_eq!(log.text(), "Reading the journal...");
    assert!(t.children().iter().any(|c| c.id == LOG));
    t.event(MAIN, key(keysym::RETURN), &mut ctx);
    let refresh = form::Event::Pressed(servicelog::REFRESH);
    assert!(matches!(
        t.svc.log.as_mut().unwrap().event(refresh),
        servicelog::Outcome::None
    ));
    assert!(LOGS.take().is_empty(), "one read at a time");
    let us = 1_764_219_292_000_000u64;
    let when = reprisefs::listing::date_text((us / 1_000_000) as i64, reprisefs::locale::is_12h());
    let json = [
        format!(r#"{{"__REALTIME_TIMESTAMP":"{us}","MESSAGE":"one"}}"#),
        format!(r#"{{"__REALTIME_TIMESTAMP":"{us}","MESSAGE":"two\n"}}"#),
    ]
    .join("\n");
    let post = |name: &str, read: Result<String, services::LogError>| services::Msg::Log {
        manager: Manager::User,
        name: name.into(),
        read,
    };
    t.svc
        .tx
        .send(post("other.service", Ok("x".into())))
        .unwrap();
    t.svc
        .tx
        .send(post("pipewire.service", Ok(json.clone())))
        .unwrap();
    t.event(MAIN, Event::Wake, &mut ctx);
    assert_eq!(
        t.svc.log.as_ref().unwrap().text(),
        format!("{when} one\n{when} two"),
        "each entry as its time and message"
    );
    t.svc
        .tx
        .send(post("pipewire.service", Ok(String::new())))
        .unwrap();
    t.event(MAIN, Event::Wake, &mut ctx);
    assert_eq!(
        t.svc.log.as_ref().unwrap().text(),
        "The journal holds no lines for this service that you may read."
    );
    t.svc
        .tx
        .send(post(
            "pipewire.service",
            Err(services::LogError::TookTooLong),
        ))
        .unwrap();
    t.event(MAIN, Event::Wake, &mut ctx);
    assert_eq!(
        t.svc.log.as_ref().unwrap().text(),
        "The journal could not be read: journalctl took too long and was stopped."
    );
    t.event(LOG, Event::CloseRequest, &mut ctx);
    assert!(t.svc.log.is_none());
}

#[test]
fn hiding_system_services_drops_their_rows_at_once() {
    let mut ctx = ctx();
    let mut t = services_mgr(&mut ctx);
    t.svc.system = true;
    t.set_system_services(false);
    assert!(
        (t.svc.rows.iter()).all(|s| s.manager == Manager::User),
        "only user services stay"
    );
}

#[test]
fn a_manager_change_reads_the_services_again_while_the_page_shows() {
    thread_local! {
        static FOLLOWS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    }
    let mut ctx = ctx();
    let mut t = loaded(&mut ctx);
    let (reads, asked) = std::sync::mpsc::channel();
    t.svc.reader = Some(reads);
    t.svc.post = Some(services::Post {
        tx: t.svc.tx.clone(),
        wake: repriseapp::Waker::noop(),
    });
    t.svc.follow = |_| FOLLOWS.set(FOLLOWS.get() + 1);
    let reread = |t: &mut TaskMgr, ctx: &mut Ctx, r| {
        t.svc.tx.send(services::Msg::Reread(r)).unwrap();
        t.event(MAIN, Event::Wake, ctx);
    };
    reread(&mut t, &mut ctx, Reread::Files);
    assert_eq!(asked.try_iter().count(), 0, "no read off the page");
    open(&mut t, &mut ctx, Page::Services);
    open(&mut t, &mut ctx, Page::Processes);
    open(&mut t, &mut ctx, Page::Services);
    assert_eq!(FOLLOWS.get(), 1, "followed from the first showing on");
    asked.try_iter().for_each(drop);
    reread(&mut t, &mut ctx, Reread::Loaded);
    let files = |asked: &std::sync::mpsc::Receiver<services::Read>| {
        asked.try_iter().map(|r| r.files).collect::<Vec<_>>()
    };
    assert_eq!(files(&asked), [false], "a unit loaded: the loaded units");
    for r in [Reread::Loaded, Reread::Files, Reread::Loaded] {
        t.svc.tx.send(services::Msg::Reread(r)).unwrap();
    }
    t.event(MAIN, Event::Wake, &mut ctx);
    assert_eq!(files(&asked), [true], "a reload: one read with the files");
}

#[test]
fn a_service_shows_its_unescaped_name_and_its_jobs_keep_the_real_one() {
    let mut ctx = ctx();
    let mut t = services_mgr(&mut ctx);
    let real = "app-reprise\\x2dbluetooth@autostart.service";
    let rows = vec![service(
        real,
        Manager::User,
        services::Status::Stopped,
        services::Startup::Manual,
    )];
    list_services(&mut t, &mut ctx, rows, true);
    assert_eq!(
        t.svc.list.view.rows[0].cells[0],
        "app-reprise-bluetooth@autostart"
    );
    click_row(&mut t, &mut ctx, 0);
    t.form_event(form::Event::Pressed(pages::START), &mut ctx);
    assert_eq!(JOBS.take(), [job(Manager::User, real, Verb::Start)]);
    t.event(MAIN, key(keysym::RETURN), &mut ctx);
    assert_eq!(LOGS.take(), [(Manager::User, real.to_string())]);
    assert_eq!(
        t.svc.log.as_ref().unwrap().title(),
        "app-reprise-bluetooth@autostart - Log"
    );
}

fn backspace() -> Event {
    Event::Key(KeyInput {
        sym: keysym::BACKSPACE,
        utf8: None,
        mods: Mods::default(),
        edit: Some(reprisekit::textedit::EditKey::Backspace { word: false }),
    })
}

pub(crate) fn typed(ch: char) -> Event {
    Event::Key(KeyInput {
        sym: ch as u32,
        utf8: Some(ch.to_string()),
        mods: Mods::default(),
        edit: Some(reprisekit::textedit::EditKey::Insert(ch.to_string())),
    })
}

fn alt(ch: char) -> Event {
    Event::Key(KeyInput {
        sym: ch as u32,
        utf8: Some(ch.to_string()),
        mods: Mods {
            alt: true,
            ..Mods::default()
        },
        edit: Some(reprisekit::textedit::EditKey::Insert(ch.to_string())),
    })
}

fn shown_names(t: &TaskMgr) -> Vec<String> {
    (t.svc.list.view.rows.iter())
        .map(|r| r.cells[0].clone())
        .collect()
}

#[test]
fn the_filter_field_narrows_the_services_and_keys_go_back_to_the_list() {
    let mut ctx = ctx();
    let mut t = services_mgr(&mut ctx);
    click_row(&mut t, &mut ctx, 2);
    assert_eq!(t.selected_service().unwrap().name, "sync.service");
    t.event(MAIN, alt('i'), &mut ctx);
    assert!(t.filter_focused(), "Alt+I is the field's access key");
    for ch in "SY".chars() {
        t.event(MAIN, typed(ch), &mut ctx);
    }
    assert_eq!(
        shown_names(&t),
        ["sync"],
        "sshd's description holds no \"sy\""
    );
    assert_eq!(t.svc.filter, "SY");
    assert_eq!(
        t.selected_service().unwrap().name,
        "sync.service",
        "the selection stays"
    );
    t.event(MAIN, key(keysym::TAB), &mut ctx);
    assert!(!t.filter_focused(), "Tab goes back to the list");
    t.form_event(form::Event::Pressed(pages::START), &mut ctx);
    assert_eq!(
        JOBS.take(),
        [job(Manager::User, "sync.service", Verb::Start)]
    );
    t.event(MAIN, key(keysym::TAB), &mut ctx);
    assert!(t.filter_focused(), "Tab from the list goes to the field");
    t.event(MAIN, backspace(), &mut ctx);
    t.event(MAIN, backspace(), &mut ctx);
    assert_eq!(shown_names(&t), ["pipewire", "sshd", "sync"]);
    click_row(&mut t, &mut ctx, 0);
    assert!(
        !t.filter_focused(),
        "a click in the list takes the keyboard back"
    );
}

#[test]
fn show_only_failed_lists_the_failed_services() {
    use services::{Startup, Status};
    let mut ctx = ctx();
    let mut t = services_mgr(&mut ctx);
    let mut rows = service_rows();
    rows.push(service(
        "broken.service",
        Manager::User,
        Status::Failed,
        Startup::Manual,
    ));
    list_services(&mut t, &mut ctx, rows, true);
    t.event(MAIN, alt('a'), &mut ctx);
    assert_eq!(t.form.checked(pages::FAILED), Some(true));
    assert_eq!(shown_names(&t), ["broken"]);
    list_services(&mut t, &mut ctx, service_rows(), true);
    assert!(shown_names(&t).is_empty(), "a read keeps the filter");
    t.event(MAIN, alt('a'), &mut ctx);
    assert_eq!(shown_names(&t), ["pipewire", "sshd", "sync"]);
}
