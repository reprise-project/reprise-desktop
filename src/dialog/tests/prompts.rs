use super::*;

#[test]
fn delete_badge_pins_at_scale_1_and_2() {
    for (s, want) in [(1, 0x056e_b456_7847_bf11u64), (2, 0x5abe_c64f_f838_2a75)] {
        let mut c = Canvas::new(32 * s as u32, 32 * s as u32);
        c.fill_rect(c.bounds(), Color::rgb(128, 128, 128));
        draw_delete_badge(&mut c, 0, 0, s);
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        for b in c.pixels.iter().flat_map(|p| p.to_le_bytes()) {
            h = (h ^ b as u64).wrapping_mul(0x100_0000_01b3);
        }
        assert_eq!(h, want, "the delete badge at scale {s}");
    }
}

#[test]
fn a_message_box_focuses_its_default_button_and_reads_its_text() {
    use reprisekit::access::accesskit::Role;
    let (t, focus) = tree_of(&Dialog::key_aid_prompt(crate::keyfilter::Aid::StickyKeys));
    let yes = t.get(focus).unwrap();
    assert_eq!(yes.role(), Role::DefaultButton);
    assert_eq!(yes.label(), Some("Yes"));
    assert!(
        t.nodes()
            .any(|(_, n)| n.value().is_some_and(|v| v.contains("Turn on StickyKeys?"))),
        "the text is there"
    );
    let (t, focus) = tree_of(&Dialog::turn_off(&power()));
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
    let Some(text) = tahoma() else { return };
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
    let c = Dialog::confirm_delete(
        "New Text Document",
        std::path::Path::new("/x"),
        false,
        false,
        false,
    );
    let l = DialogLayout::compute(&theme, &text, &c);
    assert_eq!(l.size.0, 419);
    assert_eq!(l.icon.unwrap(), Rect::new(17, 38, 32, 32));
    assert_eq!(l.lines[0].0, Rect::new(65, 38, 330, 13));
    assert_eq!(l.lines.len(), 2);
    assert_eq!(l.buttons[0], Rect::new(239, 96, 80, 23));
    assert_eq!(l.buttons[1], Rect::new(326, 96, 80, 23));
    assert_eq!(l.size.1, 133);
    let dir = std::env::temp_dir();
    let f = Dialog::confirm_delete("New Folder", &dir, true, false, false);
    assert_eq!(f.title, "Confirm Folder Delete");
    assert!(
        f.text
            .starts_with("Are you sure you want to remove the folder 'New Folder'")
    );
    let l = DialogLayout::compute(&theme, &text, &f);
    assert_eq!(l.buttons[0].y, 115);
    assert_eq!(l.size, (419, 152));
}

#[test]
fn confirm_delete_icons_asks_about_the_places_then_the_files() {
    let theme = Theme::default();
    let Some(text) = tahoma() else { return };
    let hide = vec!["home".to_string()];
    let files = vec![
        std::path::PathBuf::from("/d/a.txt"),
        std::path::PathBuf::from("/d/b.txt"),
    ];
    let mut d = Dialog::confirm_delete_icons(hide.clone(), files.clone());
    assert_eq!(d.title, "Confirm Delete");
    let l = DialogLayout::compute(&theme, &text, &d);
    assert_eq!(l.size, (512, 139));
    let ys: Vec<i32> = l.lines.iter().map(|(r, _)| r.y).collect();
    assert_eq!(ys, [33, 59, 72]);
    assert_eq!(
        l.lines[2].1,
        "To restore them later, use the Background page of Reprise Settings."
    );
    assert_eq!(l.buttons[0].y, 102);
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
    assert_eq!(l.size, (404, 126));
}

#[test]
fn confirm_hide_fits_its_text() {
    use crate::desktop::Place;
    let theme = Theme::default();
    let Some(text) = tahoma() else { return };
    let mut d = Dialog::confirm_hide(Place::Computer, "Computer");
    assert_eq!(d.title, "Confirm Delete");
    assert_eq!(d.icon.as_deref(), Some("folder-documents"));
    assert_eq!(
        d.text,
        "Are you sure you want to delete the Computer icon from your desktop?\n\n\
         To restore it later, use the Background page of Reprise Settings."
    );
    let l = DialogLayout::compute(&theme, &text, &d);
    assert_eq!(l.size, (418, 126));
    assert_eq!(l.icon.unwrap(), Rect::new(14, 33, 32, 32));
    assert_eq!(l.lines.len(), 2);
    assert_eq!((l.lines[0].0.x, l.lines[0].0.y), (65, 33));
    assert_eq!(l.lines[1].0.y, 59);
    assert_eq!(l.buttons[0], Rect::new(131, 89, 75, 23));
    assert_eq!(l.buttons[1], Rect::new(212, 89, 75, 23));
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
    assert_eq!(l.size, (707, 126));
    assert_eq!(l.buttons[0].x, 275);
    let c = draw(
        &theme,
        &text,
        &l,
        None,
        None,
        &[],
        &[],
        None,
        &docs,
        true,
        None,
        Cues::ALL,
        true,
        None,
    );
    let red = Color::rgb(255, 0, 0).argb();
    assert_eq!(c.get(14 + 3, 33 + 25), red);
    assert_eq!(c.get(14 + 8, 33 + 22), Color::rgb(255, 255, 255).argb());
    assert_ne!(c.get(14 + 3, 33 + 18), red);
}

#[test]
fn confirm_delete_trashes_on_yes_only() {
    let path = std::path::Path::new("/home/u/Desktop/it's mine.txt");
    let mut d = Dialog::confirm_delete("it's mine.txt", path, false, false, false);
    assert_eq!(d.placement, Placement::Center);
    assert!(d.text.contains("'it's mine.txt'"));
    let yes = Outcome::Delete {
        paths: vec![path.to_path_buf()],
        erase: false,
    };
    assert_eq!(d.key(Key::Enter), yes);
    assert_eq!(d.key(Key::Edit(EditKey::Insert("y".into()))), yes);
    assert_eq!(
        d.key(Key::Edit(EditKey::Insert("n".into()))),
        Outcome::Close
    );
    assert_eq!(d.key(Key::Escape), Outcome::Close);
    assert_eq!(d.press(1), Outcome::Close);
    let mut e = Dialog::confirm_delete("it's mine.txt", path, false, true, false);
    assert_eq!(e.text, "Are you sure you want to delete 'it's mine.txt'?");
    assert_eq!(e.icon.as_deref(), Some("folder"));
    assert!(matches!(
        e.style,
        Style::Classic {
            delete_badge: true,
            ..
        }
    ));
    assert_eq!(
        e.key(Key::Enter),
        Outcome::Delete {
            paths: vec![path.to_path_buf()],
            erase: true,
        }
    );
    let dir = std::env::temp_dir();
    let f = Dialog::confirm_delete("New Folder", &dir, true, true, false);
    assert_eq!(f.title, "Confirm Folder Delete");
    assert_eq!(
        f.text,
        "Are you sure you want to remove the folder 'New Folder' and all its contents?"
    );
}

#[test]
fn too_big_asks_with_the_batch_buttons() {
    use reprisefs::bincap::{Measured, TooBig};
    use reprisefs::replace::Answer;
    let big = std::path::PathBuf::from("/d/big.iso");
    let too_big = Some(Measured::unstamped(2, 1));
    let asker = TooBig::new(vec!["/d/a".into(), big.clone()], vec![None, too_big]);
    let mut d = Dialog::too_big(&big, false, asker.clone());
    assert_eq!(d.title, "Confirm File Delete");
    assert_eq!(
        d.text,
        "'big.iso' is too big for the Recycle Bin. Do you want to delete it permanently?"
    );
    let labels: Vec<&str> = d.buttons.iter().map(|b| b.label.as_str()).collect();
    assert_eq!(labels, ["&Yes", "Yes to &All", "&No", "Cancel"]);
    let answer = |answer| Outcome::TooBig {
        asker: Box::new(asker.clone()),
        answer,
    };
    assert_eq!(d.key(Key::Enter), answer(Answer::Yes));
    assert_eq!(d.press(1), answer(Answer::YesToAll));
    assert_eq!(d.press(2), answer(Answer::No));
    assert_eq!(d.press(3), Outcome::Close);
    let theme = Theme::default();
    let text = TextRenderer::load(&FontConfig::default());
    let l = DialogLayout::compute(&theme, &text, &d);
    assert_eq!(l.size.0, 419);
    assert_eq!(l.buttons.len(), 4);
    assert_eq!(l.buttons[3].right(), 419 - 13);
    for w in l.buttons.windows(2) {
        assert_eq!((w[0].width, w[1].x - w[0].right()), (80, 7));
    }

    let one = TooBig::new(vec![big.clone()], vec![too_big]);
    let d = Dialog::too_big(&big, true, one);
    assert_eq!(d.title, "Confirm Folder Delete");
    let labels: Vec<&str> = d.buttons.iter().map(|b| b.label.as_str()).collect();
    assert_eq!(labels, ["&Yes", "&No"]);
}

#[test]
fn confirm_delete_many_counts_the_items_and_trashes_them_all() {
    let paths = vec![
        std::path::PathBuf::from("/d/New Folder"),
        std::path::PathBuf::from("/d/a b.txt"),
    ];
    let mut d = Dialog::confirm_delete_many(&paths, false, BinLack::None);
    assert_eq!(d.title, "Confirm Multiple File Delete");
    assert_eq!(
        d.text,
        "Are you sure you want to send these 2 items to the Recycle Bin?"
    );
    assert_eq!(
        d.key(Key::Enter),
        Outcome::Delete {
            paths: paths.clone(),
            erase: false,
        }
    );
    let theme = Theme::default();
    let text = TextRenderer::load(&FontConfig::default());
    let l = DialogLayout::compute(&theme, &text, &d);
    assert_eq!(l.size, (419, 135));
    assert_eq!(l.icon.map(|r| r.y), Some(38));
    assert_eq!(l.lines[0].0.y, 45);
    assert_eq!(l.buttons[0], Rect::new(224, 96, 87, BUTTON_H));
    assert_eq!(l.buttons[1], Rect::new(318, 96, 87, BUTTON_H));
    let mut e = Dialog::confirm_delete_many(&paths, true, BinLack::None);
    assert_eq!(e.text, "Are you sure you want to delete these 2 items?");
    assert!(matches!(
        e.style,
        Style::Classic {
            delete_badge: true,
            ..
        }
    ));
    assert_eq!(e.key(Key::Enter), Outcome::Delete { paths, erase: true });
}

/// Yes deletes the path by its bytes: a name that is not UTF-8 reaches
/// the host as it is, not with U+FFFD in place of the stray byte.
#[test]
fn confirm_delete_keeps_a_name_that_is_not_utf8() {
    use std::os::unix::ffi::OsStrExt;
    let path = std::path::PathBuf::from(std::ffi::OsStr::from_bytes(b"/d/a\xffb.txt"));
    let mut d = Dialog::confirm_delete("a\u{fffd}b.txt", &path, false, false, false);
    assert_eq!(
        d.key(Key::Enter),
        Outcome::Delete {
            paths: vec![path.clone()],
            erase: false,
        }
    );
    let paths = vec![path, std::path::PathBuf::from("/d/c.txt")];
    let mut m = Dialog::confirm_delete_many(&paths, true, BinLack::None);
    assert_eq!(m.key(Key::Enter), Outcome::Delete { paths, erase: true });
}

#[test]
fn a_delete_off_a_drive_with_no_bin_says_so_first() {
    let one = Dialog::confirm_delete("d.txt", std::path::Path::new("/m/d.txt"), false, true, true);
    assert_eq!(
        one.text,
        "This drive has no Recycle Bin. Are you sure you want to delete 'd.txt'?"
    );
    let paths = vec!["/m/a".into(), "/m/b".into()];
    let all = Dialog::confirm_delete_many(&paths, true, BinLack::All);
    assert_eq!(
        all.text,
        "None of these items can go to the Recycle Bin. Are you sure you want to delete \
         these 2 items?"
    );
    let some = Dialog::confirm_delete_many(&paths, true, BinLack::Some);
    assert_eq!(
        some.text,
        "Some of these items cannot go to the Recycle Bin. Are you sure you want to \
         delete these 2 items?"
    );
}

#[test]
fn confirm_empty_bin_names_one_item_or_counts_them() {
    let one = Dialog::confirm_empty_bin(1, Some("notes.txt"));
    assert_eq!(one.title, "Confirm File Delete");
    assert_eq!(one.text, "Are you sure you want to delete 'notes.txt'?");
    let mut many = Dialog::confirm_empty_bin(3, Some("a"));
    assert_eq!(many.title, "Confirm Multiple File Delete");
    assert_eq!(many.text, "Are you sure you want to delete these 3 items?");
    assert_eq!(many.key(Key::Enter), Outcome::EmptyBin);
}

#[test]
fn disabled_button_is_inert() {
    let mut d = Dialog::log_off();
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
    let Some(text) = tahoma() else { return };
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
fn end_program_and_windowed_dialogs_have_a_taskbar_button() {
    assert!(Dialog::end_program("foot", 4321).on_taskbar());
    assert!(!Dialog::end_program("foot", 4321).windowed());
    assert!(!Dialog::run(&[]).on_taskbar());
    assert!(!Dialog::log_off().on_taskbar());
    assert!(!Dialog::error("Title", "Text".into()).on_taskbar());
    assert!(!Dialog::run(&[]).windowed());
    assert!(!Dialog::error("Title", "Text".into()).windowed());
}

#[test]
fn only_end_program_names_a_pid() {
    assert_eq!(
        Dialog::end_program("foot", 4321).end_program_pid(),
        Some(4321)
    );
    assert_eq!(Dialog::run(&[]).end_program_pid(), None);
    assert_eq!(Dialog::log_off().end_program_pid(), None);
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
        &[],
        &[],
        None,
        &d,
        false,
        None,
        Cues::NONE,
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
    let Some(text) = tahoma() else { return };
    for (aid, lines) in [
        (Aid::StickyKeys, 6),
        (Aid::SlowKeys, 7),
        (Aid::MouseKeys, 6),
    ] {
        let mut d = Dialog::key_aid_prompt(aid);
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
        assert_eq!(
            d.press(2),
            Outcome::Spawn(format!(
                "reprise-settings --page accessibility --focus {}",
                aid.key()
            ))
        );
    }
}
