use super::*;
use crate::procfs::MemInfo;
use crate::scopes::Scope;
use repriseapp::{Extended, MAIN, Mods};
use reprisekit::interface::Interface;
use reprisekit::text::TextRenderer;
use reprisekit::theme::Theme;

fn themed_ctx(theme: Theme) -> Ctx {
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

fn ctx() -> Ctx {
    themed_ctx(Theme::default())
}

const UID: u32 = 1000;

fn entry(id: &str, name: &str, wm_class: Option<&str>) -> AppEntry {
    let class = wm_class.map_or(String::new(), |c| format!("StartupWMClass={c}\n"));
    let text = format!(
        "[Desktop Entry]\nType=Application\nName={name}\nExec={id}\nIcon=text-editor\n{class}"
    );
    reprisefs::apps::parse_desktop(&text, &format!("{id}.desktop")).expect("an entry")
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
    }
}

fn snapshot() -> Snapshot {
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
                app: "org.kde.ark".into(),
                pids: vec![41, 42],
            },
            Scope {
                unit: "app-reprise-reprise\\x2dnotepad-2ax1x0.scope".into(),
                app: "reprise-notepad".into(),
                pids: vec![40],
            },
            Scope {
                unit: "app-reprise-mystery-3ax1x0.scope".into(),
                app: "mystery".into(),
                pids: vec![50],
            },
        ],
    }
}

fn mgr() -> TaskMgr {
    let entries = vec![
        entry("org.kde.ark", "Ark", None),
        entry("reprise-notepad", "Notepad", Some("reprise.notepad")),
    ];
    TaskMgr::new(None, entries, UID)
}

fn loaded(ctx: &mut Ctx) -> TaskMgr {
    let mut t = mgr();
    t.take(snapshot(), ctx);
    t.event(MAIN, Event::Resized, ctx);
    t
}

fn key(sym: u32) -> Event {
    Event::Key(KeyInput {
        sym,
        utf8: None,
        mods: Mods::default(),
        edit: None,
    })
}

fn click(t: &mut TaskMgr, ctx: &mut Ctx, (x, y): (i32, i32)) {
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

fn centre(r: reprisekit::raster::Rect) -> (i32, i32) {
    (r.x + r.width / 2, r.y + r.height / 2)
}

fn row_point(t: &TaskMgr, ctx: &Ctx, i: usize) -> (i32, i32) {
    let r = pages::layout(t, ctx).list.unwrap();
    let list = if t.page() == Page::Applications {
        &t.apps
    } else {
        &t.procs
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

fn open(t: &mut TaskMgr, ctx: &mut Ctx, page: Page) {
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
        window_of("reprise-notepad", &entries, "reprise.notepad"),
        "by StartupWMClass"
    );
    assert!(
        window_of("org.kde.Ark", &entries, "org.kde.ark"),
        "by id, any case"
    );
    assert!(!window_of("reprise-notepad", &entries, "firefox"));
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
    assert!(t.ask.is_none());
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
    open(&mut t, &mut ctx, Page::Processes);
    click_row(&mut t, &mut ctx, 2);
    t.event(MAIN, key(keysym::DELETE), &mut ctx);
    let Some((Ask::EndProcess { pid, start, .. }, _)) = &t.ask else {
        panic!("Delete asks before ending");
    };
    assert_eq!((*pid, *start), (42, 142));
    assert_eq!(t.children().len(), 1, "the warning opens as a window");
    t.ask = None;
    t.wait_for(Target::Process(42, 142), "helper".into());
    t.take(snapshot(), &mut ctx);
    assert!(t.ask.is_none(), "it has a few seconds to end");
    t.ending[0].since -= END_WAIT;
    t.take(snapshot(), &mut ctx);
    assert!(matches!(
        t.ask,
        Some((Ask::EndNow(Target::Process(42, 142)), _))
    ));
    t.ask = None;
    t.answered(Ask::EndNow(Target::Process(42, 142)), Some(1));
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
fn an_ended_program_is_no_longer_waited_on() {
    let mut ctx = ctx();
    let mut t = loaded(&mut ctx);
    let unit = t.app_rows[0].unit.clone();
    t.wait_for(Target::Scope(unit), "Ark".into());
    t.ending[0].since -= END_WAIT;
    let mut s = snapshot();
    s.scopes.remove(0);
    t.take(s, &mut ctx);
    assert!(t.ending.is_empty());
    assert!(t.ask.is_none());
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
}

fn sent(rx: &std::sync::mpsc::Receiver<worker::Cmd>) -> Vec<Sent> {
    rx.try_iter()
        .map(|c| match c {
            worker::Cmd::Every(d) => Sent::Every(d),
            worker::Cmd::Now => Sent::Now,
            worker::Cmd::Disk(on) => Sent::Disk(on),
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
    assert_eq!(sent(&cmds), [Sent::Disk(true)]);
    let mut s = snapshot();
    s.procs[2].disk = Some(2048);
    t.take(s, &mut ctx);
    assert_eq!(t.procs.view.rows[0].cells[5], "2 KB/s");
    assert_eq!(t.procs.view.rows[1].cells[5], "", "blank where unreadable");
    t.set_columns(columns::defaults());
    assert_eq!(sent(&cmds), [Sent::Disk(false)]);
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
fn a_hung_window_is_its_program_by_pid_and_else_by_app_id() {
    let s = snapshot();
    let entries = [
        entry("org.kde.ark", "Ark", None),
        entry("reprise-notepad", "Notepad", Some("reprise.notepad")),
    ];
    let hung = |sc: usize, id: &str, pid: u32| {
        not_responding(&s.scopes[sc], &s.scopes, &entries, &[(id.into(), pid)])
    };
    assert!(hung(0, "anything", 41), "its client runs in the scope");
    assert!(
        !hung(0, "org.kde.ark", 40),
        "a client in another scope is that program's"
    );
    assert!(
        hung(1, "reprise.notepad", 9999),
        "a client in no scope goes by its app id"
    );
    assert!(!hung(0, "reprise.notepad", 9999));
}

#[test]
fn end_task_on_a_program_not_responding_offers_end_now_at_once() {
    let mut ctx = ctx();
    let mut t = loaded(&mut ctx);
    t.set_hung(vec![("org.kde.ark".into(), 41)], &mut ctx);
    click_row(&mut t, &mut ctx, 0);
    t.event(MAIN, key(keysym::DELETE), &mut ctx);
    let unit = t.app_rows[0].unit.clone();
    assert!(matches!(&t.ask, Some((Ask::EndNow(Target::Scope(u)), _)) if *u == unit));
    assert!(
        t.ending.is_empty(),
        "nothing waits on a request it cannot answer"
    );
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
    assert_eq!(pages::core_rects(r, 2).len(), 2);
    let twelve = pages::core_rects(r, 12);
    assert_eq!(twelve[8].y, twelve[0].y + twelve[0].height + 3);
    assert!(pages::core_rects(r, 0).is_empty());
}

fn fnv(c: &Canvas) -> u64 {
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

/// Compares `got` with `tests/pins/<name>.txt`, or rewrites it under `REPRISE_BLESS`.
fn pins(name: &str, got: &[(String, u64)]) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/pins")
        .join(format!("{name}.txt"));
    let text: String = got.iter().map(|(k, h)| format!("{k} {h:016x}\n")).collect();
    if std::env::var_os("REPRISE_BLESS").is_some_and(|v| v != "0") {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, text).unwrap();
        return;
    }
    let pinned = std::fs::read_to_string(&path).unwrap_or_default();
    assert_eq!(text, pinned, "{name}: drawn hashes differ from {path:?}");
}

#[test]
fn the_pages_are_pinned_in_every_interface_at_1_and_2() {
    if !reprisekit::text::test_tahoma() {
        return;
    }
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
            t.event(MAIN, Event::Pointer(PointerInput::Leave), &mut ctx);
            for d in [1, 2] {
                t.follow_scale(MAIN, d, &mut ctx);
                let (w, h) = ctx.size;
                let mut c = Canvas::with_scale(w as u32, h as u32, d);
                ctx.at_scale(d, |ctx| t.draw(MAIN, &mut c, ctx));
                got.push((format!("{i:?} {page:?} @{d}"), fnv(&c)));
            }
        }
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
    }
    pins("pages", &got);
}

#[test]
#[ignore]
fn render_pages() {
    for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
        let mut ctx = themed_ctx(reprisekit::interfaces::theme_for(i));
        let mut t = loaded(&mut ctx);
        for k in 0..60 {
            t.cpu.push(0.4 + (f64::from(k) * 0.4).sin() * 0.3);
            t.mem.push(0.3);
            t.swap.push(0.05);
        }
        for page in Page::ALL {
            open(&mut t, &mut ctx, page);
            let (w, h) = ctx.size;
            for d in [1, 2] {
                let mut c = Canvas::with_scale(w as u32, h as u32, d);
                ctx.at_scale(d, |ctx| t.draw(MAIN, &mut c, ctx));
                reprisekit::raster::write_preview(&c, &format!("taskmgr-{i:?}-{page:?}@{d}.png"));
            }
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
    let perf = pages::layout(&t, &ctx).perf.unwrap();
    right_click(&mut t, &mut ctx, centre(perf.graph_groups[0]));
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

    right_click(&mut t, &mut ctx, centre(perf.graph_groups[1]));
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
    assert!(t.popup.is_none(), "only on the Performance page");
}
