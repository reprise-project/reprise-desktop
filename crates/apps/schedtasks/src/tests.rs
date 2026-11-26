use super::*;
use reprisekit::pointer::Interaction;
use std::sync::mpsc::{Receiver, Sender, channel};

fn test_ctx(theme: reprisekit::theme::Theme, size: (i32, i32)) -> Ctx {
    Ctx {
        theme,
        text: reprisekit::text::TextRenderer::load(&reprisekit::theme::FontConfig::default())
            .into(),
        icons: reprisekit::icontheme::IconCache::new(reprisekit::icontheme::IconTheme::load(
            "test",
            &[],
        )),
        caret_on: true,
        size,
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
    test_ctx(reprisekit::theme::Theme::default(), CLIENT)
}

/// The window with a bus whose commands the test reads and whose updates it sends.
struct Window {
    app: SchedTasks,
    cmds: Receiver<Cmd>,
    updates: Sender<Update>,
}

fn window() -> Window {
    let (cmds_tx, cmds) = channel();
    let (updates, updates_rx) = channel();
    let mut app = SchedTasks::new();
    app.is_12h = false;
    app.bus = Some(Bus {
        cmds: cmds_tx,
        updates: updates_rx,
    });
    app.sync();
    Window { app, cmds, updates }
}

impl Window {
    fn tell(&mut self, u: Update) {
        self.updates.send(u).unwrap();
        self.app.wake();
    }

    fn pick(&mut self, timer: &str) {
        let i = self.app.tasks.iter().position(|t| t.timer == timer);
        self.app.list.view.selection.set_only(i);
        self.app.sync();
    }

    fn sent(&self) -> Vec<Cmd> {
        self.cmds.try_iter().collect()
    }

    fn press(&mut self, id: &'static str) -> Reply {
        let mut ctx = ctx();
        let (x, y) = {
            let r = self.app.form.item(id).unwrap().rect;
            (r.x + 4, r.y + 4)
        };
        let press = PointerInput::Press {
            x,
            y,
            button: BTN_LEFT,
            time: 0,
            mods: Default::default(),
        };
        self.app.event(MAIN, Event::Pointer(press), &mut ctx);
        let release = PointerInput::Release {
            x,
            y,
            button: BTN_LEFT,
            mods: Default::default(),
        };
        self.app.event(MAIN, Event::Pointer(release), &mut ctx)
    }
}

fn backup() -> Task {
    Task {
        timer: "backup.timer".into(),
        service: "backup.service".into(),
        description: "Copy the photos".into(),
        timer_state: "active".into(),
        file_state: "enabled".into(),
        service_state: "inactive".into(),
        calendar: vec![("OnCalendar".into(), "*-*-* 03:00:00".into())],
        result: "success".into(),
        command: "/usr/bin/rsync -a /home/u/Photos /mnt".into(),
        file: "/home/u/.config/systemd/user/backup.timer".into(),
        ..Task::default()
    }
}

fn cleanup() -> Task {
    Task {
        timer: "cleanup.timer".into(),
        service: "cleanup.service".into(),
        description: "cleanup.service".into(),
        timer_state: "inactive".into(),
        file_state: "disabled".into(),
        service_state: "inactive".into(),
        monotonic: vec![("OnUnitActiveUSec".into(), 3_600_000_000)],
        file: "/home/u/.config/systemd/user/cleanup.timer".into(),
        ..Task::default()
    }
}

/// A task New Task made.
fn made() -> Task {
    Task {
        timer: "reprise-task-back-up.timer".into(),
        service: "reprise-task-back-up.service".into(),
        description: "Back up".into(),
        file: "/home/u/.config/systemd/user/reprise-task-back-up.timer".into(),
        ours: true,
        ..backup()
    }
}

fn key(sym: u32, text: Option<&str>) -> Event {
    Event::Key(KeyInput {
        sym,
        utf8: text.map(String::from),
        mods: Default::default(),
        edit: text.map(|t| reprisekit::textedit::EditKey::Insert(t.into())),
    })
}

fn enabled(app: &SchedTasks, id: &str) -> bool {
    match app.form.item(id).map(|i| &i.control) {
        Some(Control::Button { enabled, .. }) => *enabled,
        _ => panic!("no {id}"),
    }
}

fn label(app: &SchedTasks, id: &str) -> String {
    match app.form.item(id).map(|i| &i.control) {
        Some(Control::Button { label, .. }) => label.clone(),
        Some(Control::Label { text, .. }) => text.clone(),
        _ => panic!("no {id}"),
    }
}

#[test]
fn the_tasks_fill_the_rows_and_the_status_line() {
    let mut w = window();
    assert_eq!(label(&w.app, STATUS), "No scheduled tasks.");
    w.tell(Update::Tasks(Ok(vec![backup(), cleanup()])));
    assert_eq!(
        w.app.list.view.rows.cells,
        [
            ["Copy the photos", "Every day at 3:00", "", "Never", "Ready"].map(String::from),
            ["cleanup", "Every 1 h", "", "Never", "Disabled"].map(String::from),
        ]
    );
    assert_eq!(label(&w.app, STATUS), "2 tasks");
    w.tell(Update::Tasks(Err("Connection refused".into())));
    assert_eq!(
        label(&w.app, STATUS),
        "The service manager did not list the tasks: Connection refused"
    );
}

#[test]
fn the_buttons_follow_the_task_picked() {
    let mut w = window();
    w.tell(Update::Tasks(Ok(vec![backup(), cleanup()])));
    for id in [RUN, TOGGLE, PROPERTIES] {
        assert!(!enabled(&w.app, id), "{id} greys with nothing picked");
    }
    w.pick("backup.timer");
    assert!(enabled(&w.app, RUN) && enabled(&w.app, TOGGLE) && enabled(&w.app, PROPERTIES));
    assert_eq!(label(&w.app, TOGGLE), "&Disable");
    w.pick("cleanup.timer");
    assert_eq!(label(&w.app, TOGGLE), "&Enable");
    let running = Task {
        service_state: "active".into(),
        ..cleanup()
    };
    w.tell(Update::Tasks(Ok(vec![backup(), running])));
    assert!(!enabled(&w.app, RUN), "a running task does not start twice");
    assert_eq!(
        w.app.picked().map(|t| t.timer.as_str()),
        Some("cleanup.timer"),
        "the pick stays on its timer"
    );
}

#[test]
fn run_now_enable_and_disable_ask_the_service_manager() {
    let mut w = window();
    w.tell(Update::Tasks(Ok(vec![backup(), cleanup()])));
    w.pick("backup.timer");
    w.press(RUN);
    w.press(TOGGLE);
    w.pick("cleanup.timer");
    w.press(TOGGLE);
    assert_eq!(
        w.sent(),
        [
            Cmd::Run("backup.service".into()),
            Cmd::Turn {
                timer: "backup.timer".into(),
                file_state: "enabled".into(),
                on: false
            },
            Cmd::Turn {
                timer: "cleanup.timer".into(),
                file_state: "disabled".into(),
                on: true
            },
        ]
    );
}

fn centre(r: Rect) -> (i32, i32) {
    (r.x + r.width / 2, r.y + r.height / 2)
}

#[test]
fn a_button_let_go_off_it_does_nothing() {
    let mut w = window();
    w.tell(Update::Tasks(Ok(vec![backup()])));
    w.pick("backup.timer");
    let mut ctx = ctx();
    let (x, y) = centre(w.app.form.item(RUN).unwrap().rect);
    let (lx, ly) = centre(w.app.list_rect(&ctx));
    let events = [
        PointerInput::Press {
            x,
            y,
            button: BTN_LEFT,
            time: 0,
            mods: Default::default(),
        },
        PointerInput::Motion { x: lx, y: ly },
        PointerInput::Release {
            x: lx,
            y: ly,
            button: BTN_LEFT,
            mods: Default::default(),
        },
    ];
    for ev in events {
        w.app.event(MAIN, Event::Pointer(ev), &mut ctx);
    }
    assert_eq!(w.sent(), [], "Run Now let go over the list runs nothing");
    w.press(RUN);
    assert_eq!(w.sent(), [Cmd::Run("backup.service".into())]);
}

#[test]
fn a_button_lights_under_the_pointer_in_glass() {
    let mut w = window();
    w.tell(Update::Tasks(Ok(vec![backup()])));
    w.pick("backup.timer");
    let glass = reprisekit::interfaces::theme_for(reprisekit::interface::Interface::Glass);
    let mut ctx = test_ctx(glass, CLIENT);
    w.app
        .draw_main(&mut Canvas::new(CLIENT.0 as u32, CLIENT.1 as u32), &ctx);
    let (x, y) = centre(w.app.form.item(TOGGLE).unwrap().rect);
    let toggle = w.app.form.index_of(TOGGLE).unwrap();
    let lit = |w: &Window| w.app.form.lit(toggle, form::Light::Body);
    let over = w.app.event(
        MAIN,
        Event::Pointer(PointerInput::Motion { x, y }),
        &mut ctx,
    );
    assert!(over.redraw);
    assert_eq!(lit(&w), Interaction::Hot);
    let gone = w
        .app
        .event(MAIN, Event::Pointer(PointerInput::Leave), &mut ctx);
    assert!(gone.redraw);
    assert_eq!(lit(&w), Interaction::Idle, "nothing stays lit");
}

#[test]
fn f5_reads_the_tasks_again() {
    let mut w = window();
    let f5 = KeyInput {
        sym: keysym::F5,
        utf8: None,
        mods: Default::default(),
        edit: None,
    };
    w.app.event(MAIN, Event::Key(f5), &mut ctx());
    assert_eq!(w.sent(), [Cmd::Refresh]);
}

#[test]
fn properties_opens_a_sheet_that_closes_with_its_timer() {
    let mut w = window();
    w.tell(Update::Tasks(Ok(vec![backup(), cleanup()])));
    w.pick("backup.timer");
    w.press(PROPERTIES);
    let titles: Vec<String> = w.app.children().into_iter().map(|c| c.title).collect();
    assert_eq!(titles, ["Copy the photos Properties"]);
    w.tell(Update::Tasks(Ok(vec![cleanup()])));
    assert!(w.app.props.is_none(), "the timer is gone");
}

#[test]
fn the_window_opens_wide_enough_for_every_column() {
    for pct in [100, 200] {
        let theme = reprisekit::theme::Theme::default().with_text_size(pct);
        let mut ctx = test_ctx(theme, CLIENT);
        ctx.text_size = pct;
        ctx.size = ctx.forms().text.size(CLIENT);
        let mut w = window();
        w.tell(Update::Tasks(Ok(vec![backup(); 60])));
        let lay = w.app.list.layout(w.app.list_rect(&ctx), &ctx);
        assert!(lay.scrollbar.is_some(), "60 rows scroll at {pct}%");
        assert!(
            lay.hscroll.is_none() && lay.total_w <= lay.body.width,
            "every column shows at {pct}%: {} of {} px",
            lay.total_w,
            lay.body.width
        );
    }
}

#[test]
fn stop_stops_a_running_task() {
    let mut w = window();
    let running = Task {
        service_state: "active".into(),
        ..backup()
    };
    w.tell(Update::Tasks(Ok(vec![running, cleanup()])));
    w.pick("cleanup.timer");
    assert!(!enabled(&w.app, STOP), "cleanup is not running");
    w.pick("backup.timer");
    assert!(enabled(&w.app, STOP));
    w.press(STOP);
    assert_eq!(w.sent(), [Cmd::Stop("backup.service".into())]);
}

#[test]
fn view_output_shows_the_services_journal_read_only() {
    let mut w = window();
    w.tell(Update::Tasks(Ok(vec![backup(), cleanup()])));
    assert!(!enabled(&w.app, VIEW_OUTPUT), "greyed with nothing picked");
    w.pick("backup.timer");
    w.press(VIEW_OUTPUT);
    assert_eq!(w.sent(), [Cmd::Output("backup.service".into())]);
    let titles: Vec<String> = w.app.children().into_iter().map(|c| c.title).collect();
    assert_eq!(titles, ["Copy the photos Output"]);
    let text = |w: &Window| w.app.output.as_ref().unwrap().text();
    assert_eq!(text(&w), "Reading the journal...");
    w.tell(Update::Output(
        "cleanup.service".into(),
        Ok("not this one".into()),
    ));
    assert_eq!(text(&w), "Reading the journal...", "another unit's lines");
    w.tell(Update::Output(
        "backup.service".into(),
        Ok("Nov 26 03:00:01 rsync[12]: sent 4 files\n".into()),
    ));
    assert_eq!(text(&w), "Nov 26 03:00:01 rsync[12]: sent 4 files");
    let mut ctx = ctx();
    let a = KeyInput {
        sym: 'a' as u32,
        utf8: Some("a".into()),
        mods: Default::default(),
        edit: Some(reprisekit::textedit::EditKey::Insert("a".into())),
    };
    w.app.event(OUTPUT, Event::Key(a), &mut ctx);
    assert_eq!(
        text(&w),
        "Nov 26 03:00:01 rsync[12]: sent 4 files",
        "read only"
    );
    w.tell(Update::Output("backup.service".into(), Ok(" \n".into())));
    assert_eq!(text(&w), "The journal holds no lines for this task yet.");
    w.tell(Update::Output(
        "backup.service".into(),
        Err("No such file or directory (os error 2)".into()),
    ));
    assert_eq!(
        text(&w),
        "The journal could not be read: No such file or directory (os error 2)"
    );
    w.app.event(OUTPUT, Event::CloseRequest, &mut ctx);
    assert!(w.app.output.is_none());
}

#[test]
fn new_task_sends_the_task_and_picks_it_once_listed() {
    let mut w = window();
    w.tell(Update::Tasks(Ok(vec![backup()])));
    assert!(enabled(&w.app, NEW));
    w.app.open_new();
    let titles: Vec<String> = w.app.children().into_iter().map(|c| c.title).collect();
    assert_eq!(titles, ["New Task"]);
    let d = w.app.new_task.as_mut().unwrap();
    d.form.show_edit_text(newtask::NAME, "Back up");
    d.form.show_edit_text(newtask::PROGRAM, "/bin/sh");
    w.app.event(NEW_TASK, key(keysym::RETURN, None), &mut ctx());
    assert!(w.app.new_task.is_none(), "Enter is Create");
    assert_eq!(
        w.sent(),
        [Cmd::Create {
            task: units::NewTask {
                name: "Back up".into(),
                program: "/bin/sh".into(),
                args: Vec::new(),
                schedule: units::Schedule::Daily { at: (9, 0) },
            },
            taken: vec!["backup.timer".into()],
        }]
    );
    w.tell(Update::Created("reprise-task-back-up.timer".into()));
    w.tell(Update::Tasks(Ok(vec![backup(), made()])));
    assert_eq!(
        w.app.picked().map(|t| t.timer.as_str()),
        Some("reprise-task-back-up.timer"),
        "the new task is picked"
    );
}

#[test]
fn browse_covers_new_task() {
    let mut w = window();
    w.app.open_new();
    let d = w.app.new_task.as_mut().unwrap();
    let out = d.event(form::Event::Pressed(newtask::BROWSE));
    assert_eq!(out, newtask::Outcome::Browse);
    w.app
        .event(NEW_TASK, Event::Pointer(PointerInput::Leave), &mut ctx());
    let r = w
        .app
        .new_task
        .as_ref()
        .unwrap()
        .form
        .item(newtask::BROWSE)
        .unwrap()
        .rect;
    let mut ctx = ctx();
    for ev in [
        PointerInput::Press {
            x: r.x + 4,
            y: r.y + 4,
            button: BTN_LEFT,
            time: 0,
            mods: Default::default(),
        },
        PointerInput::Release {
            x: r.x + 4,
            y: r.y + 4,
            button: BTN_LEFT,
            mods: Default::default(),
        },
    ] {
        w.app.event(NEW_TASK, Event::Pointer(ev), &mut ctx);
    }
    assert!(w.app.browse.is_some());
    use repriseapp::Extended;
    assert_eq!(w.app.parent(BROWSE), Some(NEW_TASK));
    w.app.event(NEW_TASK, Event::CloseRequest, &mut ctx);
    assert!(w.app.browse.is_none(), "Browse leaves with New Task");
}

#[test]
fn delete_asks_first_and_only_for_a_task_new_task_made() {
    let mut w = window();
    w.tell(Update::Tasks(Ok(vec![backup(), made()])));
    w.pick("backup.timer");
    assert!(!enabled(&w.app, DELETE), "a timer New Task did not make");
    w.app.act(Act::Delete);
    assert!(w.app.asking.win().is_none());
    w.pick("reprise-task-back-up.timer");
    assert!(enabled(&w.app, DELETE));
    w.press(DELETE);
    let question = w.app.asking.win().expect("Delete asks");
    assert_eq!(w.sent(), [], "nothing before the answer");
    w.app.event(question, key(keysym::RETURN, None), &mut ctx());
    assert_eq!(w.sent(), [], "Enter is Cancel");
    w.press(DELETE);
    let question = w.app.asking.win().unwrap();
    w.app
        .event(question, key('d' as u32, Some("d")), &mut ctx());
    assert_eq!(w.sent(), [Cmd::Delete("reprise-task-back-up.timer".into())]);
}

#[test]
fn a_refusal_is_told_in_a_box() {
    let mut w = window();
    w.tell(Update::Refused(
        Cmd::Run("backup.service".into()),
        "Unit backup.service not found.".into(),
    ));
    let children = w.app.children();
    assert_eq!(children.len(), 1, "the error box");
    assert_eq!(children[0].title, "Scheduled Tasks");
}

/// The window with both tasks, the first picked, and its Properties, in each look.
fn drawn() -> Vec<(String, Canvas)> {
    let mut w = window();
    w.tell(Update::Tasks(Ok(vec![backup(), cleanup()])));
    w.pick("backup.timer");
    w.app.act(Act::Properties);
    w.app.act(Act::Output);
    w.app.new_task = Some(newtask::NewTaskBox::new(|| (2026, 11, 26, 10, 30), false));
    w.tell(Update::Output(
        "backup.service".into(),
        Ok("Nov 26 03:00:01 rsync[12]: sent 4 files\nNov 26 03:00:02 systemd[1]: Finished.".into()),
    ));
    let mut out = Vec::new();
    use reprisekit::interface::Interface;
    for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
        let theme = reprisekit::interfaces::theme_for(i);
        let mut c = Canvas::new(CLIENT.0 as u32, CLIENT.1 as u32);
        w.app.draw_main(&mut c, &test_ctx(theme.clone(), CLIENT));
        out.push((format!("window {i:?}"), c));
        let mut c = Canvas::new(props::CLIENT.0 as u32, props::CLIENT.1 as u32);
        w.app
            .draw_props(&mut c, &test_ctx(theme.clone(), props::CLIENT));
        out.push((format!("properties {i:?}"), c));
        let mut c = Canvas::new(output::CLIENT.0 as u32, output::CLIENT.1 as u32);
        w.app
            .draw_output(&mut c, &test_ctx(theme.clone(), output::CLIENT));
        out.push((format!("output {i:?}"), c));
        let mut c = Canvas::new(newtask::CLIENT.0 as u32, newtask::CLIENT.1 as u32);
        w.app
            .draw_new_task(&mut c, &test_ctx(theme, newtask::CLIENT));
        out.push((format!("new task {i:?}"), c));
    }
    out
}

fn hash(c: &Canvas) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for y in 0..c.height as i32 {
        for x in 0..c.width as i32 {
            for b in c.get(x, y).to_le_bytes() {
                h = (h ^ b as u64).wrapping_mul(0x0100_0000_01b3);
            }
        }
    }
    h
}

#[test]
fn the_window_and_its_properties_sit_on_their_pixels_in_every_look() {
    reprisekit::pins::fonts();
    let got: Vec<(String, u64)> = drawn().iter().map(|(k, c)| (k.clone(), hash(c))).collect();
    let file = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/pins/window.txt");
    reprisekit::pins::check_file(std::path::Path::new(file), &got);
}

/// Writes `target/schedtasks-*.png` for eyeballing; run with `--ignored`.
#[test]
#[ignore]
fn render_schedtasks_preview() {
    for (name, c) in drawn() {
        let name = name.to_lowercase().replace(' ', "-");
        reprisekit::raster::write_preview(&c, &format!("schedtasks-{name}.png"));
    }
}

#[test]
fn the_buttons_fit_their_labels() {
    let over = reprisekit::interfaces::form_overflows(|_, _, _| {
        let mut form = Form::new(buttons());
        if let Some(Control::Button { label, .. }) = form.item_mut(TOGGLE).map(|i| &mut i.control) {
            *label = tr!("enable");
        }
        vec![
            ("window".to_string(), Form::new(buttons())),
            ("enable".to_string(), form),
            (
                "output".to_string(),
                output::Output::new("backup.service", "Copy the photos").form,
            ),
        ]
    });
    assert!(over.is_empty(), "{}", over.join("\n"));
}
