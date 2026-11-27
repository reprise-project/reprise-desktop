use super::*;

#[test]
fn delete_badge_pins_at_scale_1_and_2() {
    let mut got = Vec::new();
    for s in [1, 2] {
        let mut c = Canvas::new(32 * s as u32, 32 * s as u32);
        c.fill_rect(c.bounds(), Color::rgb(128, 128, 128));
        draw_delete_badge(&mut c, 0, 0, s);
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        for b in c.pixels.iter().flat_map(|p| p.to_le_bytes()) {
            h = (h ^ b as u64).wrapping_mul(0x100_0000_01b3);
        }
        got.push((format!("@{s}"), h));
    }
    let pins = reprisekit::crate_dir!().join("tests/pins/delete_badge.txt");
    reprisekit::pins::check_file(&pins, &got);
}

#[test]
fn a_message_box_focuses_its_default_button_and_reads_its_text() {
    use reprisekit::access::accesskit::Role;
    let (t, focus) = tree_of(&Dialog::key_aid_prompt(
        crate::keyfilter::Aid::StickyKeys,
        true,
    ));
    let yes = t.get(focus).unwrap();
    assert_eq!(yes.role(), Role::DefaultButton);
    assert_eq!(yes.label(), Some("Yes"));
    assert!(
        t.nodes()
            .any(|(_, n)| n.value().is_some_and(|v| v.contains("Turn on StickyKeys?"))),
        "the text is there"
    );
    let (t, focus) = tree_of(&turn_off());
    assert_eq!(t.get(focus).unwrap().label(), Some("Turn Off"));
}

#[test]
fn wrap_breaks_on_words_and_inside_words_too_long_for_a_line() {
    let text = TextRenderer::load(&FontConfig::default());
    let px = 11;
    let one = text.measure("aaaa", false, px);
    assert_eq!(
        wrap(&text, "aaaa bbbb cccc", one, px),
        vec!["aaaa", "bbbb", "cccc"]
    );
    let all = text.measure("aaaa bbbb cccc", false, px);
    assert_eq!(
        wrap(&text, "aaaa bbbb cccc", all, px),
        vec!["aaaa bbbb cccc"]
    );
    assert_eq!(
        wrap(
            &text,
            "toolongtofit",
            text.measure("toolong", false, px),
            px
        ),
        vec!["toolong", "tofit"]
    );
    assert!(wrap(&text, "   ", 100, px).is_empty());
}

#[test]
fn layout_stacks_text_field_and_buttons() {
    let theme = Theme::default();
    let text = tahoma();
    let d = Dialog::run(&[]);
    let l = DialogLayout::compute(&theme, &text, &d);
    assert_eq!(l.size, (347, 179));
    assert_eq!(l.icon.unwrap(), Rect::new(14, 40, 32, 32));
    assert_eq!(l.lines.len(), 2, "the Run text wraps next to the icon");
    assert_eq!(l.lines[0].0, Rect::new(57, 40, 276, 13));
    assert_eq!(l.lines[1].0.y, 53);
    let field = l.field.unwrap();
    assert_eq!(field, Rect::new(57, 82, 275, 21));
    assert_eq!(l.field_label.unwrap().x, 14);
    assert_eq!(l.buttons.len(), 3);
    assert_eq!(l.buttons[0], Rect::new(96, 136, 75, 23));
    assert_eq!(l.buttons[1].x, 177);
    assert_eq!(l.buttons[2].right(), field.right() + 1);
    let b = l.buttons[1];
    assert_eq!(
        l.hit(&d, b.x + 1, b.y + 1, &text, theme.fonts.size_px),
        Some(DialogHit::Button(1))
    );
    assert_eq!(
        l.hit(&d, b.x - 1, b.y + 1, &text, theme.fonts.size_px),
        None
    );
    assert!(l.help.is_some() && l.drop.is_some());
    let c = yes_no("New Text Document");
    let l = DialogLayout::compute(&theme, &text, &c);
    assert_eq!(l.size.0, 419);
    assert_eq!(l.icon.unwrap(), Rect::new(17, 38, 32, 32));
    assert_eq!(l.lines[0].0, Rect::new(65, 38, 330, 13));
    assert_eq!(l.lines.len(), 2);
    assert_eq!(l.buttons[0], Rect::new(239, 96, 80, 23));
    assert_eq!(l.buttons[1], Rect::new(326, 96, 80, 23));
    assert_eq!(l.size.1, 133);
}

#[test]
fn confirm_delete_icons_asks_about_the_places_then_the_files() {
    let theme = Theme::default();
    let text = tahoma();
    let hide = vec!["home".to_string()];
    let files = vec![
        std::path::PathBuf::from("/d/a.txt"),
        std::path::PathBuf::from("/d/b.txt"),
    ];
    let mut d = Dialog::confirm_delete_icons(hide.clone(), files.clone());
    assert_eq!(d.title, "Confirm Delete");
    let l = DialogLayout::compute(&theme, &text, &d);
    let gaps: Vec<i32> = l.lines.windows(2).map(|w| w[1].0.y - w[0].0.y).collect();
    assert_eq!(
        gaps.iter().filter(|&&g| g == 2 * LINE_H).count(),
        1,
        "one blank line under the question: {gaps:?}"
    );
    assert!(gaps.iter().all(|&g| g == LINE_H || g == 2 * LINE_H));
    assert_eq!(l.buttons[0].y, l.lines.last().unwrap().0.bottom() + 17);
    let yes = Outcome::DeleteIcons {
        hide,
        files: files.clone(),
    };
    assert_eq!(d.key(Key::Enter), yes);
    assert_eq!(
        d.key(Key::Edit(EditKey::Insert("n".into()))),
        Outcome::DeleteIcons {
            hide: Vec::new(),
            files
        }
    );
    assert_eq!(d.key(Key::Escape), Outcome::Close);

    let places = Dialog::confirm_delete_icons(vec!["computer".into(), "home".into()], Vec::new());
    assert!(
        places
            .text
            .ends_with("\n\nTo restore them later, use the Background page of Reprise Settings.")
    );
    let l = DialogLayout::compute(&theme, &text, &places);
    assert_eq!(l.size, (358, 152));
}

/// Confirm Delete on system icons, one for each sentence its second paragraph can have.
fn confirm_deletes() -> Vec<(&'static str, Dialog)> {
    use crate::desktop::Place;
    vec![
        (
            "computer",
            Dialog::confirm_hide(Place::Computer, "Computer"),
        ),
        (
            "documents",
            Dialog::confirm_hide(Place::Documents, "Documents"),
        ),
        (
            "icons",
            Dialog::confirm_delete_icons(vec!["computer".into()], Vec::new()),
        ),
        (
            "icons and files",
            Dialog::confirm_delete_icons(vec!["computer".into()], vec!["/d/a".into()]),
        ),
    ]
}

/// Confirm Delete's client wraps its text at the message box cap under every interface and text
/// size, every line fitting its column, the buttons inside it.
#[test]
fn confirm_delete_wraps_at_the_message_box_cap() {
    use reprisekit::interface::Interface;
    let text = tahoma();
    for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
        for pct in [100, 150, 200] {
            let theme = reprisekit::interfaces::theme_for(i).with_text_size(pct);
            let (px, cap) = (
                theme.fonts.size_px,
                theme.text.len(reprisekit::controls::msgbox::WRAP_W),
            );
            for (name, d) in confirm_deletes() {
                let at = format!("{}, {pct}%, {name}", i.as_str());
                let l = DialogLayout::compute(&theme, &text, &d);
                let client = l.frame.expect("a framed dialog").client;
                assert!(
                    client.width <= cap,
                    "{at}: {} px within {cap}",
                    client.width
                );
                for (r, line) in &l.lines {
                    assert!(
                        text.measure(line, false, px) <= r.width,
                        "{at}: {line:?} fits"
                    );
                }
                let (first, last) = (l.buttons[0], l.buttons[l.buttons.len() - 1]);
                assert!(
                    first.x > client.x && last.right() < client.right(),
                    "{at}: buttons"
                );
            }
        }
    }
}

#[test]
fn confirm_delete_is_pinned_in_every_interface_at_1_and_2() {
    use reprisekit::interface::Interface;
    let text = tahoma();
    let mut got = Vec::new();
    for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
        let theme = reprisekit::interfaces::theme_for(i);
        for (name, d) in confirm_deletes() {
            for s in [1, 2] {
                let l = DialogLayout::compute_at(&theme, &text, &d, s);
                let c = draw(
                    &theme.scaled(s),
                    text.for_output(s),
                    &l,
                    None,
                    None,
                    &d,
                    false,
                    Lit::default(),
                    Cues::NONE,
                    true,
                    true,
                    None,
                );
                let hash = c.pixels.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, p| {
                    (h ^ u64::from(*p)).wrapping_mul(0x100_0000_01b3)
                });
                got.push((format!("{} {name} @{s}", i.as_str()), hash));
            }
        }
    }
    let pins = reprisekit::crate_dir!().join("tests/pins/confirm_delete.txt");
    reprisekit::pins::check_file(&pins, &got);
}

#[test]
fn confirm_hide_fits_its_text() {
    use crate::desktop::Place;
    let theme = Theme::default();
    let text = tahoma();
    let mut d = Dialog::confirm_hide(Place::Computer, "Computer");
    assert_eq!(d.title, "Confirm Delete");
    assert_eq!(d.icon.as_deref(), Some("folder-documents"));
    assert_eq!(
        d.text,
        "Are you sure you want to delete the Computer icon from your desktop?\n\n\
         To restore it later, use the Background page of Reprise Settings."
    );
    let l = DialogLayout::compute(&theme, &text, &d);
    assert_eq!(l.icon.unwrap(), Rect::new(14, 33, 32, 32));
    assert_eq!((l.lines[0].0.x, l.lines[0].0.y), (65, 33));
    let yes = Outcome::HideSystemIcon("computer".into());
    assert_eq!(d.key(Key::Enter), yes);
    assert_eq!(d.key(Key::Edit(EditKey::Insert("y".into()))), yes);
    assert_eq!(
        d.key(Key::Edit(EditKey::Insert("n".into()))),
        Outcome::Close
    );
    assert_eq!(d.key(Key::Escape), Outcome::Close);
    let folder_restore = Some(
        "The contents of the folder will not be deleted. You can restore this icon \
         to the desktop on the Background page of Reprise Settings.",
    );
    let home = Dialog::confirm_hide(Place::Home, "u's Home");
    assert_eq!(home.text.split("\n\n").nth(1), folder_restore);
    let docs = Dialog::confirm_hide(Place::Documents, "Documents");
    assert_eq!(docs.text.split("\n\n").nth(1), folder_restore);
    let l = DialogLayout::compute(&theme, &text, &docs);
    let c = draw(
        &theme,
        &text,
        &l,
        None,
        None,
        &docs,
        true,
        None,
        Cues::ALL,
        true,
        true,
        None,
    );
    let red = Color::rgb(255, 0, 0).argb();
    assert_eq!(c.get(14 + 3, 33 + 25), red);
    assert_eq!(c.get(14 + 8, 33 + 22), Color::rgb(255, 255, 255).argb());
    assert_ne!(c.get(14 + 3, 33 + 18), red);
}

#[test]
fn disabled_button_is_inert() {
    let mut d = log_off();
    assert!(!d.buttons[0].enabled);
    assert_eq!(d.press(0), Outcome::Nothing);
    assert_eq!(
        d.key(Key::Edit(EditKey::Insert("s".into()))),
        Outcome::Nothing
    );
    assert_eq!(d.press(1), Outcome::Act(Action::Exit));
    assert_eq!(d.press(2), Outcome::Close);
}

#[test]
fn keys_move_the_focus_along_a_message_boxs_buttons() {
    let focused = |d: &Dialog| d.buttons.iter().position(|b| b.default);
    let mut d = Dialog::end_program("foot", 4321);
    assert_eq!(focused(&d), Some(1), "Cancel has the focus at first");
    assert_eq!(d.key(Key::Tab { back: false }), Outcome::Redraw);
    assert_eq!(focused(&d), Some(0), "Tab wraps round to End Now");
    assert_eq!(d.key(Key::Tab { back: true }), Outcome::Redraw);
    assert_eq!(focused(&d), Some(1), "Shift+Tab wraps back to Cancel");
    assert_eq!(d.key(Key::Right), Outcome::Nothing, "Right stops at Cancel");
    assert_eq!(d.key(Key::Down), Outcome::Nothing, "Down stops at Cancel");
    assert_eq!(d.key(Key::Left), Outcome::Redraw);
    assert_eq!(focused(&d), Some(0));
    assert_eq!(d.key(Key::Up), Outcome::Nothing, "Up stops at End Now");
    // The view sends Left and Right as editing keys.
    let right = EditKey::Right {
        select: false,
        word: false,
    };
    assert_eq!(d.key(Key::Edit(right)), Outcome::Redraw);
    assert_eq!(focused(&d), Some(1), "Right moves on to Cancel");
    let left = EditKey::Left {
        select: false,
        word: false,
    };
    assert_eq!(d.key(Key::Edit(left)), Outcome::Redraw);
    assert_eq!(focused(&d), Some(0), "Left moves back to End Now");
    assert_eq!(d.key(Key::Enter), Outcome::EndProgram(4321));

    let mut three = Dialog::error("Title", "Text".into());
    three.buttons = vec![
        button("&Yes", DialogAction::Cancel, true),
        button("&No", DialogAction::Cancel, false),
        button("Cancel", DialogAction::Cancel, false),
    ];
    three.buttons[1].enabled = false;
    three.key(Key::Tab { back: false });
    assert_eq!(focused(&three), Some(2), "Tab skips the greyed No");
    three.key(Key::Left);
    assert_eq!(focused(&three), Some(0), "Left skips the greyed No");
}

#[test]
fn space_presses_a_message_boxs_focused_button() {
    let space = || Key::Edit(EditKey::Insert(" ".into()));
    let mut d = Dialog::end_program("foot", 4321);
    assert_eq!(d.key(space()), Outcome::Close, "Space presses Cancel");
    let mut d = Dialog::end_program("foot", 4321);
    d.key(Key::Tab { back: false });
    assert_eq!(d.key(space()), Outcome::EndProgram(4321));
    let mut d = Dialog::end_program("foot", 4321);
    d.key(Key::Tab { back: false });
    assert_eq!(d.key(Key::Space), Outcome::EndProgram(4321));

    let mut run = Dialog::run(&[]);
    assert_eq!(run.key(space()), Outcome::Redraw);
    let field = run.field.as_ref().and_then(Field::edit);
    assert_eq!(field.map(|e| e.text.as_str()), Some(" "));
}

#[test]
fn end_program_lays_out_its_template() {
    let theme = Theme::default();
    let text = tahoma();
    let mut d = Dialog::end_program("signal-desktop", 4321);
    assert_eq!(d.title, "End Program - signal-desktop");
    let l = DialogLayout::compute(&theme, &text, &d);
    assert_eq!(l.size, (354, 243));
    assert_eq!(l.icon.unwrap(), Rect::new(13, 32, 32, 32));
    let tops: Vec<i32> = l.lines.iter().map(|(r, _)| r.y).collect();
    assert_eq!(tops, vec![35, 74, 87, 120, 133]);
    assert!(l.lines.iter().all(|(r, _)| r.x == 58 && r.right() == 336));
    assert_eq!(l.lines[0].1, "This program has stopped responding.");
    assert_eq!(
        l.buttons,
        vec![Rect::new(188, 204, 69, 23), Rect::new(266, 204, 69, 23),]
    );
    assert_eq!(d.key(Key::Enter), Outcome::Close);
    assert_eq!(d.key(Key::Escape), Outcome::Close);
    assert_eq!(d.press(0), Outcome::EndProgram(4321));
}

#[test]
fn only_end_program_has_a_taskbar_button() {
    assert!(Dialog::end_program("foot", 4321).on_taskbar());
    assert!(!Dialog::run(&[]).on_taskbar());
    assert!(!log_off().on_taskbar());
    assert!(!Dialog::error("Title", "Text".into()).on_taskbar());
}

#[test]
fn only_end_program_names_a_pid() {
    assert_eq!(
        Dialog::end_program("foot", 4321).end_program_pid(),
        Some(4321)
    );
    assert_eq!(Dialog::run(&[]).end_program_pid(), None);
    assert_eq!(log_off().end_program_pid(), None);
}

/// The advances a second dialog font gives at 11 px where they differ
/// from the UI font's, for tests that stand a stub in for it: a wider
/// E and N than the default UI face's.
fn shell_dlg_advance(c: char) -> i32 {
    match c {
        'E' => 7,
        'N' | 'w' => 8,
        ' ' | 'i' | 'l' | '.' | ',' => 3,
        _ => 6,
    }
}

#[test]
fn end_program_sets_its_text_and_buttons_in_the_second_dialog_font() {
    let theme = Theme::default();
    let text = TextRenderer::load(&FontConfig::default())
        .with_shell_dlg(TextRenderer::stub(11, shell_dlg_advance));
    let d = Dialog::end_program("signal-desktop", 4321);
    assert_eq!(d.font(), DialogFont::ShellDlg);
    let l = DialogLayout::compute(&theme, &text, &d);
    assert_eq!(l.size, (354, 243));
    let second = text.dialog(DialogFont::ShellDlg);
    assert_eq!(second.measure("End Now", false, 11), 44);
    let width = 354 - 18 - 58;
    let want = wrap(second, &d.text, width, 11);
    let got: Vec<&str> = l.lines.iter().map(|(_, s)| s.as_str()).collect();
    assert_eq!(
        got,
        want.iter()
            .filter(|s| !s.is_empty())
            .map(String::as_str)
            .collect::<Vec<_>>()
    );
    assert!(l.lines.iter().all(|(r, _)| r.x == 57), "{:?}", l.lines);
    let ui = DialogLayout::compute(&theme, &TextRenderer::load(&FontConfig::default()), &d);
    assert!(ui.lines.iter().all(|(r, _)| r.x == 58), "{:?}", ui.lines);
    assert_eq!(l.buttons[0], Rect::new(188, 204, 69, 23));
    let c = draw(
        &theme,
        &text,
        &l,
        None,
        None,
        &d,
        false,
        None,
        Cues::NONE,
        true,
        true,
        None,
    );
    let b = l.buttons[0].inset(3);
    let pens: Vec<i32> = (b.x..b.right())
        .filter(|&x| (b.y..b.bottom()).any(|y| c.get(x, y) == 0xFF00_0000))
        .map(|x| x - l.buttons[0].x)
        .collect();
    assert_eq!(pens, vec![12, 19, 25, 34, 42, 48]);
}

#[test]
fn a_key_aid_prompt_asks_on_its_fixed_template() {
    use crate::keyfilter::Aid;
    let theme = Theme::default();
    let text = tahoma();
    for (aid, lines) in [
        (Aid::StickyKeys, 6),
        (Aid::SlowKeys, 7),
        (Aid::MouseKeys, 6),
    ] {
        let mut d = Dialog::key_aid_prompt(aid, true);
        assert_eq!(d.title, aid.name());
        assert_eq!(d.placement, Placement::CenterAs(180));
        let l = DialogLayout::compute(&theme, &text, &d);
        assert_eq!(l.size, (410, 163), "{aid:?}");
        assert_eq!(l.icon, None);
        assert_eq!(l.lines.len(), lines, "{aid:?}: {:?}", l.lines);
        assert_eq!(l.lines[0].0, Rect::new(18, 30, 374, LINE_H));
        assert_eq!(l.lines[1].0.y, 43);
        assert_eq!(
            l.buttons,
            vec![
                Rect::new(78, 129, 75, 23),
                Rect::new(165, 129, 75, 23),
                Rect::new(252, 129, 75, 23),
            ]
        );
        assert_eq!(d.key(Key::Enter), Outcome::TurnOn(aid));
        assert_eq!(d.key(Key::Escape), Outcome::Close);
        assert_eq!(d.press(1), Outcome::Close);
        assert_eq!(d.press(2), Outcome::Spawn(aid.settings_argv().join(" ")));
    }
}

#[test]
fn a_key_aid_prompt_without_settings_greys_settings_and_leaves_s_inert() {
    use crate::keyfilter::Aid;
    let mut d = Dialog::key_aid_prompt(Aid::StickyKeys, false);
    assert_eq!(d.buttons[2].label, "&Settings...");
    assert!(!d.buttons[2].enabled, "Settings... is greyed");
    assert_eq!(d.press(2), Outcome::Nothing);
    assert_eq!(
        d.key(Key::Edit(EditKey::Insert("s".into()))),
        Outcome::Nothing
    );
    assert_eq!(d.key(Key::Enter), Outcome::TurnOn(Aid::StickyKeys));
}

#[test]
fn the_script_question_runs_on_enter_and_offers_notepad() {
    use reprisefs::program::{Program, ScriptOpen};
    let path = std::path::PathBuf::from("/home/u/setup.sh");
    let mut d = Dialog::ask_run(path.clone(), ScriptOpen::Terminal, Program::Script);
    let labels: Vec<&str> = d.buttons.iter().map(|b| b.label.as_str()).collect();
    assert_eq!(labels, ["&Run", "Open in &Notepad", "Cancel"]);
    assert_eq!(
        d.key(Key::Enter),
        Outcome::Script {
            path: path.clone(),
            run: Some(ScriptOpen::Terminal),
            program: Program::Script,
        }
    );
    assert_eq!(
        d.press(1),
        Outcome::Script {
            path,
            run: None,
            program: Program::Script,
        }
    );
    assert_eq!(d.key(Key::Escape), Outcome::Close);
}

#[test]
fn the_program_question_offers_run_or_cancel_with_cancel_on_enter() {
    use reprisefs::program::{Program, ScriptOpen};
    let path = std::path::PathBuf::from("/media/u/STICK/setup.exe");
    let mut binary = Dialog::ask_run(path.clone(), ScriptOpen::Run, Program::Binary);
    let mut typed = Dialog::ask_open(path.clone(), Some("wine.desktop".parse().unwrap()), true);
    for d in [&mut binary, &mut typed] {
        let labels: Vec<&str> = d.buttons.iter().map(|b| b.label.as_str()).collect();
        assert_eq!(labels, ["&Run", "Cancel"]);
        assert!(d.text.contains("is a program"), "{}", d.text);
        assert_eq!(d.key(Key::Enter), Outcome::Close, "Cancel is the default");
    }
    assert_eq!(
        binary.press(0),
        Outcome::Script {
            path: path.clone(),
            run: Some(ScriptOpen::Run),
            program: Program::Binary,
        }
    );
    assert_eq!(
        typed.press(0),
        Outcome::OpenRunnable {
            path,
            with: Some("wine.desktop".parse().unwrap()),
            recent: true,
        }
    );
}

/// A press on a box's button moves the focus and the default ring to it. Dragged off and let go,
/// it acts on nothing and keeps them: Enter presses it and a reader hears it as the default.
#[test]
fn a_press_moves_the_focus_and_default_to_the_pressed_button() {
    use reprisekit::access::accesskit::Role;
    let path = std::path::PathBuf::from("/d/a.iso");
    let mut d = Dialog::ask_mount(path.clone());
    assert!(d.buttons[1].default, "Cancel starts as the default");
    assert!(d.press_button(0));
    let defaults: Vec<bool> = d.buttons.iter().map(|b| b.default).collect();
    assert_eq!(defaults, [true, false], "the held Mount has the ring");
    assert!(d.drag_button(None));
    assert_eq!(d.release_button(None), Some(Outcome::Redraw));
    let (t, focus) = tree_of(&d);
    let mount = t.get(focus).unwrap();
    assert_eq!(mount.label(), Some("Mount"));
    assert_eq!(mount.role(), Role::DefaultButton);
    let cancel = t
        .nodes()
        .find(|(_, n)| n.label() == Some("Cancel"))
        .unwrap();
    assert_eq!(cancel.1.role(), Role::Button);
    assert_eq!(d.key(Key::Enter), Outcome::MountImage(path));
}

#[test]
fn the_undo_erase_questions_yes_undoes_again_deleting_the_item() {
    let mut undos = reprisefs::undo::UndoStack::default();
    undos.push(reprisefs::undo::Undoable::New("/d/New".into(), None));
    let taken = undos.take().unwrap();
    let entry = undos.put_back(taken, 0, None).unwrap();
    let mut d = Dialog::ask_undo_erase("Delete it?".into(), entry);
    assert_eq!(
        d.buttons[1].action,
        DialogAction::Cancel,
        "No keeps the entry"
    );
    assert_eq!(d.key(Key::Enter), Outcome::UndoErase(entry));
}

/// In Run a press on a button takes the focus from the field, as Tab would; let go off it, Enter
/// presses that button.
#[test]
fn a_press_on_runs_button_takes_the_focus_from_the_field() {
    let mut d = Dialog::run(&history());
    assert!(d.press_button(1));
    assert_eq!(d.button_focus, Some(1));
    assert_eq!(d.release_button(None), Some(Outcome::Redraw));
    assert_eq!(d.button_focus, Some(1));
    assert_eq!(d.key(Key::Enter), Outcome::Close);
}

/// A press on a greyed button holds nothing and leaves the focus where it was.
#[test]
fn a_press_on_a_greyed_button_keeps_the_focus() {
    let mut d = Dialog::key_aid_prompt(crate::keyfilter::Aid::StickyKeys, false);
    assert!(!d.press_button(2));
    let defaults: Vec<bool> = d.buttons.iter().map(|b| b.default).collect();
    assert_eq!(defaults, [true, false, false]);
}
