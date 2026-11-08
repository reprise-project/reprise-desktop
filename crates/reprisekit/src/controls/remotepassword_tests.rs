use super::*;
use crate::controls::form::{Control, Form};
use crate::interface::Interface;
use crate::interfaces;
use crate::raster::Canvas;
use crate::text::TextRenderer;
use crate::theme::Theme;

fn facts(manager: Manager) -> Facts {
    Facts {
        manager,
        yours: true,
        access: Access::Allowed,
        busy: false,
    }
}

fn running(has: bool) -> Facts {
    facts(Manager::Running(Answer::Has(has)))
}

const MANAGERS: [Manager; 4] = [
    Manager::Stopped,
    Manager::Running(Answer::Silent),
    Manager::Running(Answer::Has(false)),
    Manager::Running(Answer::Has(true)),
];

fn outside(group: &str) -> Access {
    Access::NotInGroup(group.into())
}

/// `f` under every answer the login manager can give on whether you may set one.
fn switches(f: &Facts) -> Vec<Facts> {
    [
        Access::Allowed,
        Access::RemoteLoginsOff,
        outside("reprise-remote"),
        Access::Root,
    ]
    .into_iter()
    .map(|access| Facts {
        access,
        ..f.clone()
    })
    .collect()
}

fn shown(f: &Facts) -> Section {
    section(f).expect("the section shows")
}

#[test]
fn without_the_login_manager_the_section_is_hidden() {
    for yours in [true, false] {
        for f in switches(&facts(Manager::Missing)) {
            assert_eq!(section(&Facts { yours, ..f }), None);
        }
    }
}

#[test]
fn another_users_entry_shows_only_a_greyed_note() {
    for manager in MANAGERS {
        for f in switches(&facts(manager)) {
            let f = Facts { yours: false, ..f };
            assert_eq!(
                shown(&f),
                Section {
                    status: None,
                    actions: Vec::new(),
                    note: Some(NOT_YOURS.into()),
                    note_greyed: true,
                },
                "{f:?}"
            );
        }
    }
}

#[test]
fn a_stopped_login_manager_greys_create_and_says_it_is_not_running() {
    for f in switches(&facts(Manager::Stopped)) {
        let s = shown(&f);
        assert_eq!(s.status, None);
        assert_eq!(s.actions, [(Action::Create, false)]);
        assert_eq!(s.note.as_deref(), Some(NO_MANAGER));
        assert!(!s.note_greyed);
    }
}

#[test]
fn a_silent_login_manager_greys_create_and_says_it_asks_again() {
    for f in switches(&facts(Manager::Running(Answer::Silent))) {
        let s = shown(&f);
        assert_eq!(s.status, None);
        assert_eq!(s.actions, [(Action::Create, false)]);
        assert_eq!(s.note.as_deref(), Some(SILENT));
    }
}

#[test]
fn in_the_group_with_remote_logins_on_every_action_is_lit() {
    assert_eq!(
        shown(&running(false)),
        Section {
            status: Some(HAS_NONE),
            actions: vec![(Action::Create, true)],
            note: None,
            note_greyed: false,
        }
    );
    assert_eq!(
        shown(&running(true)),
        Section {
            status: Some(HAS_ONE),
            actions: vec![(Action::Change, true), (Action::Remove, true)],
            note: None,
            note_greyed: false,
        }
    );
}

#[test]
fn with_remote_logins_off_create_and_change_grey_and_say_so() {
    for has in [false, true] {
        let f = Facts {
            access: Access::RemoteLoginsOff,
            ..running(has)
        };
        let s = shown(&f);
        assert_eq!(s.note.as_deref(), Some(REMOTE_OFF), "{f:?}");
        let set = if has { Action::Change } else { Action::Create };
        assert_eq!(s.actions[0], (set, false), "{f:?}");
    }
}

#[test]
fn while_you_cannot_set_one_the_status_claims_no_logon() {
    for access in [Access::RemoteLoginsOff, outside("staff-rdp")] {
        for (has, status) in [(false, HAS_NONE_SHORT), (true, HAS_ONE_SHORT)] {
            let f = Facts {
                access: access.clone(),
                ..running(has)
            };
            assert_eq!(shown(&f).status, Some(status), "{f:?}");
        }
    }
}

#[test]
fn roots_own_entry_is_unavailable_and_says_why() {
    for has in [false, true] {
        let s = shown(&Facts {
            access: Access::Root,
            ..running(has)
        });
        assert_eq!(s.status, None);
        assert_eq!(s.actions, [(Action::Create, false)]);
        assert_eq!(s.note.as_deref(), Some(ROOT));
    }
}

#[test]
fn a_group_name_past_the_longest_allowed_is_cut_short() {
    let long = "a".repeat(GROUP_CHARS + 8);
    let note = shown(&Facts {
        access: outside(&long),
        ..running(false)
    })
    .note
    .expect("a note");
    let cut = format!("{}...", "a".repeat(GROUP_CHARS - 3));
    assert!(note.contains(&format!(" the {cut} group.")), "{note}");
    let exact = "b".repeat(GROUP_CHARS);
    let note = shown(&Facts {
        access: outside(&exact),
        ..running(false)
    })
    .note
    .expect("a note");
    assert!(note.contains(&format!(" the {exact} group.")), "{note}");
}

#[test]
fn outside_the_group_create_and_change_grey_and_name_the_group() {
    let f = Facts {
        access: outside("staff-rdp"),
        ..running(false)
    };
    let s = shown(&f);
    assert_eq!(s.actions, [(Action::Create, false)]);
    assert_eq!(
        s.note.as_deref(),
        Some(
            "You can't set a Remote Desktop password because you aren't in the staff-rdp group. \
             An administrator can add you."
        )
    );
    let s = shown(&Facts {
        access: outside("staff-rdp"),
        ..running(true)
    });
    assert_eq!(s.actions, [(Action::Change, false), (Action::Remove, true)]);
}

#[test]
fn removing_your_own_password_stays_lit_whatever_the_switch_and_group() {
    for f in switches(&running(true)) {
        if f.access == Access::Root {
            continue;
        }
        assert!(shown(&f).actions.contains(&(Action::Remove, true)), "{f:?}");
    }
}

#[test]
fn a_change_under_way_greys_every_action() {
    for has in [false, true] {
        let s = shown(&Facts {
            busy: true,
            ..running(has)
        });
        assert!(s.actions.iter().all(|(_, lit)| !lit), "{s:?}");
        assert_eq!(s.note, None);
    }
}

#[test]
fn a_press_on_an_action_reads_as_it() {
    assert_eq!(action(&Event::Pressed(CREATE)), Some(Action::Create));
    assert_eq!(action(&Event::Pressed(CHANGE)), Some(Action::Change));
    assert_eq!(action(&Event::Pressed(REMOVE)), Some(Action::Remove));
    assert_eq!(action(&Event::Pressed("ok")), None);
    assert_eq!(action(&Event::Default), None);
}

/// The width of an account sheet's page inside its 16 px margins.
const W: i32 = 366;

/// Each state the section is pinned and audited in, by name.
fn states() -> Vec<(&'static str, Facts)> {
    vec![
        ("none set", running(false)),
        ("set", running(true)),
        (
            "outside the group",
            Facts {
                access: outside("reprise-remote"),
                ..running(false)
            },
        ),
        (
            "set outside the group",
            Facts {
                access: outside("reprise-remote"),
                ..running(true)
            },
        ),
        (
            "remote logins off",
            Facts {
                access: Access::RemoteLoginsOff,
                ..running(false)
            },
        ),
        ("login manager stopped", facts(Manager::Stopped)),
        (
            "login manager silent",
            facts(Manager::Running(Answer::Silent)),
        ),
        (
            "root",
            Facts {
                access: Access::Root,
                ..running(false)
            },
        ),
        (
            "another user",
            Facts {
                yours: false,
                ..running(true)
            },
        ),
    ]
}

/// Room around the group box, so its title and frame draw whole.
const M: i32 = 8;

/// The section for `f` at `(M, M)`, and the bottom of the canvas it draws on.
fn form_of(f: &Facts) -> (Form, i32) {
    let (items, bottom) = items(&shown(f), M, M, W);
    (Form::new(items), bottom + M)
}

#[test]
fn every_item_sits_inside_the_group_box() {
    for (name, f) in states() {
        let (form, bottom) = form_of(&f);
        let group = form.items[0].rect;
        assert_eq!(group, Rect::new(M, M, W, bottom - 2 * M), "{name}");
        for it in &form.items[1..] {
            let r = it.rect;
            assert!(
                r.x >= group.x + INSET && r.right() <= group.right() - INSET,
                "{name}: {} at {r:?}",
                it.id
            );
            assert!(
                r.y >= group.y + TOP && r.bottom() <= group.bottom() - BOTTOM,
                "{name}: {}",
                it.id
            );
        }
    }
}

#[test]
fn another_users_note_is_greyed_and_yours_is_not() {
    let note = |f: &Facts| form_of(f).0.item(NOTE).map(|i| i.control.clone());
    assert!(matches!(
        note(&Facts {
            yours: false,
            ..running(true)
        }),
        Some(Control::GreyedLines(_))
    ));
    assert!(matches!(
        note(&facts(Manager::Stopped)),
        Some(Control::Lines(_))
    ));
}

/// The longest group name the note shows whole, and a directory's name past it.
fn long_groups() -> Vec<(&'static str, Facts)> {
    let spaced = "remote desktop users of floor 3b";
    assert_eq!(spaced.len(), GROUP_CHARS);
    let word = "x".repeat(70);
    vec![
        (
            "outside a long-named group",
            Facts {
                access: outside(spaced),
                ..running(true)
            },
        ),
        (
            "outside a group named past the cut",
            Facts {
                access: outside(&word),
                ..running(true)
            },
        ),
    ]
}

#[test]
fn the_section_fits_its_text_under_every_interface() {
    let over = interfaces::form_overflows(|_, _, _| {
        (states().into_iter().chain(long_groups()))
            .map(|(name, f)| (name.to_string(), form_of(&f).0))
            .collect()
    });
    assert!(over.is_empty(), "{}", over.join("\n"));
}

/// Buttons do not widen for their text, so each action's label must fit its button with
/// room for the focus rect.
#[test]
fn every_action_label_fits_its_button_under_every_interface() {
    let over = interfaces::overflows(|_, theme, text| {
        let px = theme.fonts.size_px;
        [Action::Create, Action::Change, Action::Remove]
            .into_iter()
            .filter_map(|a| {
                let label = crate::text::split_mnemonic(a.label()).0;
                let by = text.ink_width(&label, false, px) + 12 - theme.text.len(ACTION_W);
                (by > 0).then_some((a.label().to_string(), a.id(), by))
            })
            .collect()
    });
    assert!(over.is_empty(), "{}", over.join("\n"));
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
fn the_section_pins_its_pixels_in_every_state_and_look() {
    crate::pins::fonts();
    let mut got = Vec::new();
    for (look, base) in looks() {
        for (state, f) in states() {
            let (form, bottom) = form_of(&f);
            for s in [1, 2] {
                let theme = base.scaled(s);
                let text = TextRenderer::load(&theme.fonts);
                let p = theme.paint();
                let mut c = Canvas::new(((W + 2 * M) * s) as u32, (bottom * s) as u32);
                c.fill_rect(c.bounds(), p.cs.button_face);
                crate::controls::form::draw(&mut c, &text, &p, &form, true, theme.fonts.size_px);
                got.push((format!("{look} {state} {s}"), fnv(&c)));
            }
        }
    }
    crate::pins::check("remote_password", &got);
}
