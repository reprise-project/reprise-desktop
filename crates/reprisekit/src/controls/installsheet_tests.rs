use super::*;
use crate::controls::form::Control;
use crate::interface::Interface;
use crate::interfaces;
use crate::theme::{FontConfig, Theme};

fn rend() -> (TextRenderer, u32) {
    let text = TextRenderer::load(&FontConfig::default());
    (text, Theme::default().fonts.size_px)
}

fn sheet(stage: Stage) -> Sheet {
    let mut icon = Canvas::new(32, 32);
    icon.fill_rect(
        Rect::new(4, 4, 24, 24),
        crate::theme::Color::rgb(52, 120, 200),
    );
    Sheet {
        verb: Verb::Uninstall,
        name: "Foot".into(),
        icon: Some(icon),
        facts: vec![
            ("Package:".into(), "foot".into()),
            ("Version:".into(), "1.28.0-2".into()),
            ("Size:".into(), "812 KB".into()),
            ("Source:".into(), "CachyOS packages".into()),
        ],
        note: "Uninstalling it also removes foot-terminfo.".into(),
        stage,
    }
}

const REFUSAL: &str = "Uninstalling it would remove glibc, pacman (needed by the system) and \
                       reprise (part of Reprise).";

fn running(cancellable: bool) -> Stage {
    Stage::Running {
        fraction: Some(0.4),
        doing: "Removing foot".into(),
        cancellable,
    }
}

fn buttons(f: &Form) -> Vec<(&'static str, bool)> {
    f.items
        .iter()
        .filter_map(|i| match i.control {
            Control::Button { enabled, .. } => Some((i.id, enabled)),
            _ => None,
        })
        .collect()
}

fn lines(f: &Form, id: &str) -> Vec<String> {
    match &f.item(id).map(|i| &i.control) {
        Some(Control::Lines(l)) => l.clone(),
        Some(Control::Label { text, .. }) => vec![text.clone()],
        other => panic!("{id} is {other:?}"),
    }
}

#[test]
fn the_sheet_lays_out_on_its_grid_and_keeps_its_size_through_the_stages() {
    let (text, px) = rend();
    let s = sheet(Stage::Asking { ready: true });
    let (w, h) = client(&s);
    assert_eq!((w, h), (400, 52 + 4 * 16 + 8 + 39 + 14 + 23 + 12));
    let f = form(&s, &text, px, TextUnits::ONE);
    let r = |f: &Form, id: &str| f.item(id).expect("the item exists").rect;
    assert_eq!((r(&f, "icon").x, r(&f, "icon").y), (12, 12));
    assert_eq!((r(&f, HEAD).x, r(&f, HEAD).y), (56, 14));
    assert_eq!((r(&f, "fact1").x, r(&f, "fact1").y), (56, 68));
    assert_eq!((r(&f, "value1").x, r(&f, "value1").y), (120, 68));
    let (act, cancel) = (r(&f, ACT), r(&f, CANCEL));
    assert_eq!((cancel.right(), cancel.bottom()), (w - 12, h - 12));
    assert_eq!(act.right() + 6, cancel.x);
    for stage in [
        Stage::Reading,
        running(true),
        Stage::Done,
        Stage::Failed("x".into()),
        Stage::Refused,
    ] {
        assert_eq!(client(&sheet(stage.clone())), (w, h), "{stage:?}");
    }
    let done = form(&sheet(Stage::Done), &text, px, TextUnits::ONE);
    assert_eq!(r(&done, CLOSE), cancel, "Close takes Cancel's place");
}

#[test]
fn each_stage_says_where_it_stands_and_offers_its_buttons() {
    let (text, px) = rend();
    let f = |stage| form(&sheet(stage), &text, px, TextUnits::ONE);
    let reading = f(Stage::Reading);
    assert_eq!(lines(&reading, HEAD), ["Looking up Foot..."]);
    assert_eq!(buttons(&reading), [(ACT, false), (CANCEL, true)]);
    let asking = f(Stage::Asking { ready: true });
    assert_eq!(lines(&asking, HEAD), ["Do you want to uninstall Foot?"]);
    assert_eq!(
        lines(&asking, NOTE),
        ["Uninstalling it also removes foot-terminfo."]
    );
    assert_eq!(buttons(&asking), [(ACT, true), (CANCEL, true)]);
    assert_eq!(
        buttons(&f(Stage::Asking { ready: false })),
        [(ACT, false), (CANCEL, true)]
    );
    let run = f(running(false));
    assert_eq!(lines(&run, HEAD), ["Uninstalling Foot..."]);
    assert_eq!(lines(&run, DOING), ["Removing foot"]);
    assert_eq!(buttons(&run), [(ACT, false), (CANCEL, false)]);
    let failed = f(Stage::Failed("Failed to obtain authentication.".into()));
    assert_eq!(lines(&failed, HEAD), ["Foot was not uninstalled."]);
    assert_eq!(lines(&failed, NOTE), ["Failed to obtain authentication."]);
    assert_eq!(buttons(&failed), [(CLOSE, true)]);
    let mut refused = sheet(Stage::Refused);
    refused.note = REFUSAL.into();
    let refused = form(&refused, &text, px, TextUnits::ONE);
    assert_eq!(lines(&refused, HEAD), ["Reprise cannot uninstall Foot."]);
    assert_eq!(lines(&refused, NOTE).join(" "), REFUSAL);
    assert_eq!(buttons(&refused), [(CLOSE, true)]);
    assert_eq!(refused.focus().map(|i| refused.items[i].id), Some(CLOSE));
    let mut install = sheet(Stage::Done);
    install.verb = Verb::Install;
    let done = form(&install, &text, px, TextUnits::ONE);
    assert_eq!(lines(&done, HEAD), ["Foot was installed."]);
    assert_eq!(install.title(), "Install Software");
}

/// Enter presses Cancel while asking, so only the Install or Uninstall button goes ahead; once it
/// ended every way out closes.
#[test]
fn enter_never_goes_ahead_and_the_focus_rests_on_the_way_out() {
    let (text, px) = rend();
    let asking = sheet(Stage::Asking { ready: true });
    let f = form(&asking, &text, px, TextUnits::ONE);
    assert!(f.is_consent());
    assert_eq!(f.focus().map(|i| f.items[i].id), Some(CANCEL));
    assert_eq!(f.default_button().map(|i| f.items[i].id), Some(CANCEL));
    assert_eq!(asking.choice(&Event::Pressed(ACT)), Some(Choice::Go));
    assert_eq!(asking.choice(&Event::Default), Some(Choice::Cancel));
    assert_eq!(asking.choice(&Event::Cancel), Some(Choice::Cancel));
    assert_eq!(asking.choice(&Event::Redraw), None);
    let done = sheet(Stage::Done);
    let f = form(&done, &text, px, TextUnits::ONE);
    assert_eq!(f.focus().map(|i| f.items[i].id), Some(CLOSE));
    for ev in [Event::Default, Event::Cancel, Event::Pressed(CLOSE)] {
        assert_eq!(done.choice(&ev), Some(Choice::Close), "{ev:?}");
    }
}

#[test]
fn a_running_sheet_reads_its_progress_out() {
    let (text, px) = rend();
    let s = sheet(running(true));
    let f = form(&s, &text, px, TextUnits::ONE);
    let mut tree = Tree::window("", client(&s));
    let focus = access(&s, &f, &mut tree, &text, px, TextUnits::ONE);
    assert!(focus.is_some());
    let bar = tree
        .nodes()
        .find(|(_, n)| n.role() == Role::ProgressIndicator)
        .map(|(_, n)| n.numeric_value());
    assert_eq!(bar, Some(Some(40.0)));
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

fn looks() -> Vec<(String, Theme)> {
    let mut out = vec![
        ("Classic".to_string(), Theme::default()),
        (
            "Blue".to_string(),
            interfaces::blue_theme(crate::blue::Variant::ALL[0]),
        ),
    ];
    for mode in [crate::glass::Mode::Light, crate::glass::Mode::Dark] {
        let g = crate::glass::Settings {
            mode,
            ..Default::default()
        };
        let t = interfaces::theme_with(Interface::Glass, Default::default(), &g);
        out.push((format!("Glass {mode:?}"), t));
    }
    out
}

#[test]
fn the_sheet_pins_its_pixels_in_every_stage_and_look() {
    crate::pins::fonts();
    let stages = [
        ("asking", Stage::Asking { ready: true }),
        ("running", running(true)),
        ("done", Stage::Done),
        (
            "failed",
            Stage::Failed("Failed to obtain authentication.".into()),
        ),
        ("refused", Stage::Refused),
    ];
    let mut got = Vec::new();
    for (look, base) in looks() {
        for (name, stage) in &stages {
            let mut s = sheet(stage.clone());
            if *stage == Stage::Refused {
                s.note = REFUSAL.into();
            }
            for scale in [1, 2] {
                let theme = base.scaled(scale);
                let text = TextRenderer::load(&theme.fonts);
                let px = theme.fonts.size_px;
                let f = form(&s, &text, px, TextUnits::ONE);
                let (w, h) = client(&s);
                let mut c = Canvas::new((w * scale) as u32, (h * scale) as u32);
                draw(&mut c, &text, &theme.paint(), &s, &f, px);
                got.push((format!("{look} {name} {scale}"), fnv(&c)));
            }
        }
    }
    crate::pins::check("installsheet", &got);
}

#[test]
fn the_bar_fills_its_share_and_shows_only_while_running() {
    let (text, px) = rend();
    let theme = Theme::default();
    let drawn = |stage: Stage| {
        let s = sheet(stage);
        let f = form(&s, &text, px, TextUnits::ONE);
        let (w, h) = client(&s);
        let mut c = Canvas::new(w as u32, h as u32);
        draw(&mut c, &text, &theme.paint(), &s, &f, px);
        (c, bar_rect(&s))
    };
    let (c, bar) = drawn(running(true));
    let chunk = theme.colors.hilight.argb();
    let mid = bar.y + bar.height / 2;
    let at = |frac: f64| c.get(bar.x + (bar.width as f64 * frac) as i32, mid);
    assert_eq!(at(0.2), chunk, "the first 40% is filled");
    assert_ne!(at(0.8), chunk, "the rest is not");
    let (c, bar) = drawn(Stage::Asking { ready: true });
    let row = |y| (bar.x..bar.right()).any(|x| c.get(x, y) == chunk);
    assert!(!row(mid), "no bar while asking");
}
