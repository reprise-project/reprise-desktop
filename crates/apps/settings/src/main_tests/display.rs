use super::*;

/// DP-1 listed with a smaller resolution picked, Apply or OK pressed
/// and taken, so the question is up.
fn question_up(tag: &str, press: displayhost::Press) -> (SettingsApp, Ctx) {
    let mut app = app(tag);
    let mut ctx = test_ctx();
    app.event(
        MAIN,
        Event::Heads(vec![displaypage::test_head("DP-1")]),
        &mut ctx,
    );
    app.display.model.pick_resolution(1);
    app.start_display(press, &mut ctx.heads);
    let asked = ctx.heads.take();
    assert_eq!(asked.len(), 1, "the mode is asked for");
    let r = app.event(
        MAIN,
        Event::HeadsApplied {
            serial: asked[0].0,
            ok: true,
        },
        &mut ctx,
    );
    assert!(r.tick, "the countdown runs on the main window's ticks");
    assert!(
        app.children()
            .iter()
            .any(|c| c.id == displayhost::MONITOR_WIN),
        "Monitor Settings is up"
    );
    (app, ctx)
}

#[test]
fn apply_greys_once_a_profile_pick_is_written() {
    let mut app = app("profile-apply");
    let mut ctx = test_ctx();
    app.event(
        MAIN,
        Event::Heads(vec![displaypage::test_head("DP-1")]),
        &mut ctx,
    );
    let srgb = reprisefs::icc::Listed {
        path: "/p/srgb.icc".into(),
        name: "sRGB".into(),
    };
    app.display.model.set_profiles(vec![srgb], Vec::new());
    app.display.model.pick_profile(1);
    app.sync_apply();
    let lit = |app: &SettingsApp| match &app.sheet.pages[0].item(propsheet::APPLY).unwrap().control
    {
        form::Control::Button { enabled, .. } => *enabled,
        c => panic!("Apply is a button, not {c:?}"),
    };
    assert!(lit(&app));
    assert!(app.apply());
    app.start_display(displayhost::Press::Apply, &mut ctx.heads);
    assert!(!lit(&app), "the pick is written");
}

#[test]
fn the_questions_close_box_puts_the_old_mode_back_and_keeps_the_sheet() {
    let (mut app, mut ctx) = question_up("question-close", displayhost::Press::Apply);
    let r = app.event(displayhost::MONITOR_WIN, Event::CloseRequest, &mut ctx);
    assert!(!r.quit);
    assert!(app.display.flow.question.is_none());
    let back = ctx.heads.take();
    assert_eq!(back.len(), 1);
    assert_eq!(
        back[0].1,
        vec![("DP-1".to_string(), displaypage::test_head("DP-1").state())]
    );
    let r = app.event(
        MAIN,
        Event::HeadsApplied {
            serial: back[0].0,
            ok: true,
        },
        &mut ctx,
    );
    assert!(!r.quit, "the sheet stays open after the put-back");
    assert!(!app.path.exists(), "nothing written");
}

#[test]
fn yes_after_ok_writes_the_output_and_quits() {
    let (mut app, mut ctx) = question_up("question-yes", displayhost::Press::Ok);
    let q = app.display.flow.question.as_ref().expect("the question");
    let yes = q.dialog.form.item(monitordlg::YES).expect("a Yes button");
    let r = yes.placed(&ctx.forms().controls());
    let (x, y) = (r.x + r.width / 2, r.y + r.height / 2);
    let mods = Mods::default();
    let press = PointerInput::Press {
        x,
        y,
        button: BTN_LEFT,
        time: 0,
        mods,
    };
    let release = PointerInput::Release {
        x,
        y,
        button: BTN_LEFT,
        mods,
    };
    let win = displayhost::MONITOR_WIN;
    assert!(!app.event(win, Event::Pointer(press), &mut ctx).quit);
    assert!(app.event(win, Event::Pointer(release), &mut ctx).quit);
    let text = std::fs::read_to_string(&app.path).expect("config written");
    assert!(
        text.contains("name = \"DP-1\"") && text.contains("mode = \"1280x720@60\""),
        "DP-1's [[outputs]] entry has the new mode: {text}"
    );
}

/// Two outputs side by side; the second dragged under the first and
/// made the primary one. Apply sends the primary to the desktop at once
/// and asks for both places in one request; Yes writes both.
#[test]
fn a_kept_arrangement_writes_every_place_and_the_primary() {
    let mut app = app("arrangement");
    let mut ctx = test_ctx();
    let mut b = displaypage::test_head("HDMI-A-1");
    b.position = (1920, 0);
    app.event(
        MAIN,
        Event::Heads(vec![displaypage::test_head("DP-1"), b]),
        &mut ctx,
    );
    let mut well = reprisekit::controls::arrange::Arrange::new(app.display.model.tiles(), 1);
    well.tiles[1].pos = (0, 1080);
    app.display.model.set_arrangement(&well);
    app.display.model.make_primary();
    app.start_display(displayhost::Press::Apply, &mut ctx.heads);
    assert_eq!(app.display.sent_primary.as_deref(), Some("HDMI-A-1"));
    let asked = ctx.heads.take();
    assert_eq!(asked.len(), 1, "one request");
    let places: Vec<_> = asked[0]
        .1
        .iter()
        .map(|(n, c)| (n.as_str(), c.position))
        .collect();
    assert_eq!(
        places,
        vec![("DP-1", Some((0, 0))), ("HDMI-A-1", Some((0, 1080)))]
    );
    app.event(
        MAIN,
        Event::HeadsApplied {
            serial: asked[0].0,
            ok: true,
        },
        &mut ctx,
    );
    let effects = app.display.flow.answer(true, &mut ctx.heads);
    app.display_effects(effects);
    let text = std::fs::read_to_string(&app.path).unwrap();
    assert_eq!(
        outputwrite::read_primary(&text),
        None,
        "the primary lives in the saved layout"
    );
    assert!(
        text.contains("position = [0, 0]") && text.contains("position = [0, 1080]"),
        "{text}"
    );
}

/// Whether Apply is lit on the Display page.
fn apply_lit(app: &SettingsApp) -> bool {
    let page = &app.sheet.pages[Page::Display.index()];
    match page.item(propsheet::APPLY).map(|i| &i.control) {
        Some(form::Control::Button { enabled, .. }) => *enabled,
        _ => panic!("Apply is a button"),
    }
}

#[test]
fn ok_waits_for_the_primary_call_and_a_failed_one_keeps_the_sheet_with_a_box() {
    let mut app = app("primary-pending");
    let mut ctx = test_ctx();
    let heads = vec![
        displaypage::test_head("DP-1"),
        displaypage::test_head("HDMI-A-1"),
    ];
    app.event(MAIN, Event::Heads(heads), &mut ctx);
    let mut well = reprisekit::controls::arrange::Arrange::new(app.display.model.tiles(), 1);
    well.tiles[1].pos = (0, 1080);
    app.display.model.set_arrangement(&well);
    app.display.model.make_primary();
    let r = app.start_display(displayhost::Press::Ok, &mut ctx.heads);
    assert!(!r.quit);
    assert_eq!(app.display.sent_primary.as_deref(), Some("HDMI-A-1"));
    let asked = ctx.heads.take();
    let r = app.event(
        MAIN,
        Event::HeadsApplied {
            serial: asked[0].0,
            ok: true,
        },
        &mut ctx,
    );
    assert!(!r.quit);
    let effects = app.display.flow.answer(true, &mut ctx.heads);
    let r = app.display_effects(effects);
    assert!(!r.quit, "OK waits for the primary call");
    assert!(app.display.quit_after_primary);
    assert_eq!(app.primary_done(), None, "still running");
    app.sync_apply();
    assert!(!apply_lit(&app), "Apply greyed while it runs");

    let answer = app.display.primary_answer.take().unwrap();
    answer.send(Err("refused")).unwrap();
    assert_eq!(app.primary_done(), Some(false), "the sheet stays");
    let text = app.error.as_ref().map(|b| b.fields.text.clone());
    assert_eq!(text.as_deref(), Some("refused"));
    assert!(app.display.primary_job.is_none());
}

#[test]
fn a_refused_vrr_on_ok_says_so_and_keeps_the_sheet() {
    let mut app = app("vrr-refused");
    let mut ctx = test_ctx();
    // The test output supports VRR and runs without it.
    app.event(
        MAIN,
        Event::Heads(vec![displaypage::test_head("DP-1")]),
        &mut ctx,
    );
    app.display.model.set_vrr(true);
    let r = app.start_display(displayhost::Press::Ok, &mut ctx.heads);
    assert!(!r.quit, "waits for the answer");
    let asked = ctx.heads.take();
    assert_eq!(asked.len(), 1, "only VRR is asked for");
    let r = app.event(
        MAIN,
        Event::HeadsApplied {
            serial: asked[0].0,
            ok: false,
        },
        &mut ctx,
    );
    assert!(!r.quit, "the sheet stays open");
    let text = app.error.as_ref().map(|b| b.fields.text.clone());
    assert_eq!(text.as_deref(), Some(displayflow::VRR_REFUSED));
    assert!(
        app.children().iter().any(|c| c.id == ERROR_WIN),
        "the error box is up"
    );
    assert!(!app.display.model.pick().unwrap().vrr, "the box unticks");
}

/// After Only 2 the display switcher saved HDMI-A-1 as the pair's primary while config.toml
/// marks DP-1: the page ticks HDMI-A-1, and its Extend puts HDMI-A-1 on the left.
#[test]
fn the_page_takes_the_primary_from_the_saved_layout_over_config() {
    use repriseconf::displays::{Layout, Layouts, Monitor, MonitorLayout, monitor_ids};
    let mut app = app("primary-from-layout");
    let mut ctx = test_ctx();
    let config = "[[outputs]]\nname = \"DP-1\"\nprimary = true\n";
    std::fs::write(&app.path, config).unwrap();
    let mut off = displaypage::test_head("DP-1");
    off.enabled = false;
    off.current = None;
    let heads = vec![off, displaypage::test_head("HDMI-A-1")];
    let monitors: Vec<Monitor> = (heads.iter())
        .map(|h| Monitor {
            connector: &h.name,
            make: &h.make,
            model: &h.model,
            serial: &h.serial,
        })
        .collect();
    let ids = monitor_ids(&monitors);
    let saved = |enabled, primary| MonitorLayout {
        enabled,
        mode: None,
        position: (0, 0),
        scale: 1.0,
        transform: repriseconf::config::Transform::Normal,
        primary,
    };
    let (parsed, _) = repriseconf::config::Config::parse_lenient(config).unwrap();
    let seen = repriseconf::displays::outputs_fingerprint(&parsed.outputs);
    let mut layouts = Layouts::parse("");
    let pair = vec![
        (ids[0].clone(), saved(false, false)),
        (ids[1].clone(), saved(true, true)),
    ];
    layouts.put(Layout::new(pair), &seen, &seen);
    app.display.layouts = app.path.with_file_name("displays.toml");
    std::fs::write(&app.display.layouts, layouts.render()).unwrap();
    app.event(MAIN, Event::Heads(heads), &mut ctx);
    let d = &mut app.display.model;
    assert_eq!(d.heads()[d.primary_index()].name, "HDMI-A-1");
    d.pick_projection(0);
    let places: Vec<_> = (d.changes().unwrap().set.iter())
        .map(|(n, c)| (n.clone(), c.position))
        .collect();
    assert_eq!(
        places,
        [
            ("DP-1".to_string(), Some((1920, 0))),
            ("HDMI-A-1".to_string(), Some((0, 0)))
        ]
    );
}
