use super::*;
use crate::controls::form::{Control, Form};
use crate::interface::Interface;
use crate::interfaces;
use crate::raster::Canvas;
use crate::text::TextRenderer;
use crate::theme::Theme;

/// Feeds `keys` as typed: each char through [`input_for`], `\n` as Enter, `\x1b` as Escape,
/// `\x08` as Backspace.
fn typed(calc: &mut Calc, keys: &str) {
    for ch in keys.chars() {
        let sym = match ch {
            '\n' => keysym::RETURN,
            '\x1b' => keysym::ESCAPE,
            '\x08' => keysym::BACKSPACE,
            c => c as u32,
        };
        let input = input_for(sym, Some(ch).filter(|c| !c.is_control()), false)
            .unwrap_or_else(|| panic!("{ch:?} is a key"));
        calc.input(input);
    }
}

fn after(mode: Mode, keys: &str) -> Calc {
    let mut calc = Calc::new(mode);
    typed(&mut calc, keys);
    calc
}

fn shows(mode: Mode, keys: &str) -> String {
    let calc = after(mode, keys);
    match calc.error {
        Some(e) => format!("{e:?}"),
        None => calc.display(),
    }
}

#[test]
fn standard_works_each_operator_as_it_is_pressed() {
    assert_eq!(shows(Mode::Standard, "2+3*4="), "20");
    assert_eq!(
        shows(Mode::Standard, "2+3*"),
        "5",
        "× shows the running value"
    );
    assert_eq!(shows(Mode::Standard, "10-4-3\n"), "3");
}

#[test]
fn scientific_works_the_expression_with_precedence() {
    assert_eq!(shows(Mode::Scientific, "2+3*4="), "14");
    assert_eq!(shows(Mode::Scientific, "2^3^2="), "512", "^ to the right");
    assert_eq!(shows(Mode::Scientific, "(2+3)*4="), "20");
    assert_eq!(
        shows(Mode::Scientific, "2*(3+4"),
        "4",
        "an open group shows its operand"
    );
    assert_eq!(
        shows(Mode::Scientific, "2*(3+4)"),
        "7",
        ") shows the group's value"
    );
    assert_eq!(
        shows(Mode::Scientific, "2*(3+4\n"),
        "14",
        "= closes open groups"
    );
    assert_eq!(
        shows(Mode::Scientific, "2(3+4)="),
        "14",
        "a number before ( multiplies"
    );
    let sci = after(Mode::Scientific, "2^10\n(1+2)*3\n");
    assert_eq!(sci.display(), "9", "an answer before ( is replaced");
    assert_eq!(shows(Mode::Scientific, "0-2^2="), "-4");
}

#[test]
fn scientific_answers_agree_with_the_launcher() {
    for expr in ["2+3*4", "1/3", "0.1+0.2", "2^70", "7*6-1/8"] {
        let calc = after(Mode::Scientific, &format!("{expr}="));
        assert_eq!(Some(calc.display()), calc::answer(expr), "{expr}");
    }
}

#[test]
fn an_operator_pressed_after_another_replaces_it() {
    assert_eq!(shows(Mode::Standard, "6+*2="), "12");
    assert_eq!(shows(Mode::Scientific, "6+*2="), "12");
}

#[test]
fn equals_repeats_the_last_operation_in_standard() {
    assert_eq!(shows(Mode::Standard, "2+3=="), "8");
    assert_eq!(
        shows(Mode::Standard, "5+=="),
        "15",
        "a missing operand is the left one"
    );
}

#[test]
fn typing_edits_the_entry() {
    assert_eq!(shows(Mode::Standard, "0007"), "7", "leading zeros go");
    assert_eq!(shows(Mode::Standard, ".5"), "0.5");
    assert_eq!(shows(Mode::Standard, "1.2.3"), "1.23", "one point");
    assert_eq!(shows(Mode::Standard, "123\x08"), "12");
    assert_eq!(shows(Mode::Standard, "1\x08"), "0");
    assert_eq!(
        shows(Mode::Standard, &"9".repeat(20)),
        "9".repeat(MAX_DIGITS)
    );
    assert_eq!(
        shows(Mode::Standard, "12+3\x1b"),
        "0",
        "Escape clears everything"
    );
    let mut calc = after(Mode::Standard, "12+34");
    calc.input(Input::ClearEntry);
    typed(&mut calc, "5=");
    assert_eq!(calc.display(), "17", "CE clears only the entry");
}

#[test]
fn functions_apply_to_the_shown_value() {
    assert_eq!(shows(Mode::Standard, "16@"), "4");
    assert_eq!(shows(Mode::Standard, "4r"), "0.25");
    assert_eq!(
        shows(Mode::Standard, "200+10%"),
        "20",
        "a percent of the left operand"
    );
    assert_eq!(shows(Mode::Standard, "200+10%="), "220");
    assert_eq!(shows(Mode::Standard, "50*10%="), "5");
    let mut calc = after(Mode::Standard, "5");
    calc.input(Input::Func(Func::Negate));
    assert_eq!(calc.display(), "-5");
    typed(&mut calc, "+8=");
    assert_eq!(calc.display(), "3");
    assert_eq!(shows(Mode::Scientific, "5!"), "120");
    assert_eq!(shows(Mode::Scientific, "9q"), "81");
    assert_eq!(shows(Mode::Scientific, "1000l"), "3");
    assert_eq!(shows(Mode::Scientific, "en"), "1");
    assert_eq!(shows(Mode::Scientific, "p"), "3.14159265359");
}

#[test]
fn trig_follows_the_angle_unit() {
    assert_eq!(shows(Mode::Scientific, "30s"), "0.5");
    assert_eq!(
        shows(Mode::Scientific, "180s"),
        "0",
        "a right angle is exact"
    );
    assert_eq!(shows(Mode::Scientific, "60o"), "0.5");
    assert_eq!(shows(Mode::Scientific, "45t"), "1");
    assert_eq!(shows(Mode::Scientific, "90t"), "Invalid");
    let mut calc = Calc::new(Mode::Scientific);
    calc.angle = Angle::Radians;
    typed(&mut calc, "ps");
    assert_eq!(calc.display(), "0");
    calc.angle = Angle::Grads;
    typed(&mut calc, "\x1b100s");
    assert_eq!(calc.display(), "1");
}

#[test]
fn scientific_keys_do_nothing_in_standard() {
    let mut calc = after(Mode::Standard, "5");
    assert!(!calc.input(Input::Func(Func::Factorial)));
    assert!(!calc.input(Input::Op('^')));
    assert_eq!(calc.display(), "5");
}

#[test]
fn errors_show_until_cleared_or_typed_over() {
    assert_eq!(shows(Mode::Standard, "5/0="), "DivideByZero");
    assert_eq!(shows(Mode::Scientific, "5/0="), "DivideByZero");
    assert_eq!(shows(Mode::Standard, "0r"), "DivideByZero");
    assert_eq!(shows(Mode::Standard, "1-2=@"), "Invalid");
    assert_eq!(shows(Mode::Scientific, "171!"), "Invalid");
    assert_eq!(shows(Mode::Scientific, "2.5!"), "Invalid");
    assert_eq!(shows(Mode::Scientific, "0n"), "Invalid");
    assert_eq!(shows(Mode::Scientific, "10^999="), "Invalid");
    let mut calc = after(Mode::Standard, "5/0=");
    assert!(!calc.input(Input::Op('+')), "an operator waits for a clear");
    assert_eq!(calc.copy_text(), None);
    typed(&mut calc, "7");
    assert_eq!((calc.error, calc.display()), (None, "7".into()));
}

#[test]
fn the_memory_stores_adds_recalls_and_clears() {
    let mut calc = after(Mode::Standard, "5");
    calc.input(Input::MemStore);
    typed(&mut calc, "3");
    calc.input(Input::MemAdd);
    assert_eq!(calc.memory, Some(8.0));
    typed(&mut calc, "1");
    assert_eq!(calc.display(), "1", "a digit after M+ starts a new number");
    calc.input(Input::MemSub);
    assert_eq!(calc.memory, Some(7.0));
    typed(&mut calc, "\x1b");
    assert_eq!(calc.memory, Some(7.0), "C keeps the memory");
    typed(&mut calc, "2*");
    calc.input(Input::MemRecall);
    typed(&mut calc, "=");
    assert_eq!(calc.display(), "14");
    calc.input(Input::MemClear);
    assert_eq!(calc.memory, None);
    assert!(!calc.input(Input::MemClear));
}

#[test]
fn the_memory_keys_are_ctrl_letters() {
    for (c, input) in [
        ('l', Input::MemClear),
        ('r', Input::MemRecall),
        ('m', Input::MemStore),
        ('p', Input::MemAdd),
        ('q', Input::MemSub),
    ] {
        assert_eq!(input_for(c as u32, Some(c), true), Some(input));
    }
    assert_eq!(
        input_for('c' as u32, Some('c'), true),
        None,
        "Ctrl+C is the host's"
    );
    assert_eq!(
        input_for(0xffc6, None, false),
        Some(Input::Func(Func::Negate))
    );
    assert_eq!(
        input_for(keysym::KP_ENTER, None, false),
        Some(Input::Equals)
    );
    assert_eq!(
        input_for(keysym::DELETE, None, false),
        Some(Input::ClearEntry)
    );
}

#[test]
fn equals_adds_a_history_line_to_reuse() {
    let mut calc = after(Mode::Standard, "2+3=");
    typed(&mut calc, "10/4=");
    let rows: Vec<String> = calc.history.iter().map(Line::text).collect();
    assert_eq!(rows, ["2 + 3 = 5", "10 ÷ 4 = 2.5"]);
    assert_eq!(calc.expression(), "10 ÷ 4 =");
    typed(&mut calc, "\x1b");
    calc.reuse(0);
    typed(&mut calc, "*2=");
    assert_eq!(calc.display(), "10");
    assert_eq!(calc.history.len(), 3, "C keeps the history");
    let sci = after(Mode::Scientific, "2*(3+4\n");
    assert_eq!(sci.history[0].text(), "2 × ( 3 + 4 ) = 14");
    let closed = after(Mode::Scientific, "(1+2)*3=");
    assert_eq!(
        closed.history[0].text(),
        "( 1 + 2 ) × 3 = 9",
        "a closed group stays in the line"
    );
    assert_eq!(shows(Mode::Scientific, "(1+2)(3+4)="), "21");
    assert_eq!(
        shows(Mode::Scientific, "(1+2)4="),
        "12",
        "a number after ) multiplies"
    );
    let mut long = Calc::new(Mode::Standard);
    for _ in 0..HISTORY + 5 {
        typed(&mut long, "1+1=");
    }
    assert_eq!(long.history.len(), HISTORY);
}

#[test]
fn the_expression_line_shows_the_operation_in_progress() {
    assert_eq!(after(Mode::Standard, "12+").expression(), "12 +");
    assert_eq!(after(Mode::Standard, "12+3*").expression(), "15 ×");
    assert_eq!(after(Mode::Scientific, "2+(3*").expression(), "2 + ( 3 ×");
    assert_eq!(after(Mode::Standard, "12").expression(), "");
}

#[test]
fn paste_shows_a_number_or_works_an_expression_in_scientific() {
    let mut calc = Calc::new(Mode::Standard);
    assert!(calc.paste(" 42.5\n"));
    assert_eq!(calc.display(), "42.5");
    assert!(!calc.paste("2+3"), "Standard takes only numbers");
    assert!(!calc.paste("hello"));
    let mut sci = Calc::new(Mode::Scientific);
    assert!(sci.paste("2 + 3*4"));
    assert_eq!(sci.display(), "14");
    assert_eq!(sci.copy_text().as_deref(), Some("14"));
}

#[test]
fn switching_modes_keeps_the_value_memory_and_history() {
    let mut calc = after(Mode::Standard, "6*7=");
    calc.input(Input::MemStore);
    typed(&mut calc, "+1");
    calc.set_mode(Mode::Scientific);
    assert_eq!(calc.display(), "1");
    assert_eq!(calc.expression(), "", "the operation in progress goes");
    assert_eq!((calc.memory, calc.history.len()), (Some(42.0), 1));
}

#[test]
fn every_pad_button_maps_to_its_input() {
    let calc = Calc::new(Mode::Scientific);
    let f = form(&calc, true);
    for it in &f.items {
        if matches!(it.control, Control::Button { .. } | Control::Radio { .. }) {
            assert!(press_of(it.id).is_some(), "{} presses something", it.id);
        }
    }
    assert_eq!(press_of("7"), Some(Press::Input(Input::Digit(7))));
    assert_eq!(press_of(RADIANS), Some(Press::Angle(Angle::Radians)));
    assert_eq!(press_of("display"), None);
}

#[test]
fn the_form_grows_by_mode_and_history() {
    let (sw, h) = client(Mode::Standard, false);
    let (cw, _) = client(Mode::Scientific, false);
    let (hw, _) = client(Mode::Standard, true);
    assert!(cw > sw && hw > sw, "{sw} {cw} {hw}");
    for (mode, history) in [(Mode::Standard, false), (Mode::Scientific, true)] {
        let f = form(&Calc::new(mode), history);
        let (w, _) = client(mode, history);
        for it in &f.items {
            assert!(
                it.rect.right() <= w - MARGIN_FOR_TEST && it.rect.bottom() <= h - MARGIN_FOR_TEST,
                "{} inside the client",
                it.id
            );
        }
    }
}

const MARGIN_FOR_TEST: i32 = 8;

#[test]
fn the_history_list_follows_the_lines_newest_in_view() {
    let mut calc = after(Mode::Standard, "1+1=2+2=");
    let mut f = form(&calc, true);
    let list = |f: &Form| match &f.item(HISTORY_LIST).expect("the list").control {
        Control::ListBox { list, .. } => (list.items.clone(), list.top),
        _ => unreachable!(),
    };
    assert_eq!(list(&f), (vec!["2 + 2 = 4".into(), "1 + 1 = 2".into()], 0));
    if let Some(Control::ListBox { list, .. }) = f.item_mut(HISTORY_LIST).map(|i| &mut i.control) {
        list.selected = Some(1);
    }
    assert_eq!(
        picked_line(&f, &calc),
        Some(0),
        "the bottom row is the oldest line"
    );
    calc.clear_history();
    follow(&mut f, &calc);
    assert_eq!(list(&f).0, Vec::<String>::new());
}

#[test]
fn the_pad_fits_its_text_under_every_interface() {
    let over = interfaces::form_overflows(|_, _, _| {
        let calc = after(Mode::Scientific, "1+1=");
        vec![
            ("standard".into(), form(&Calc::new(Mode::Standard), false)),
            ("scientific".into(), form(&calc, true)),
        ]
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

fn drawn(theme: &Theme, calc: &Calc, history: bool, s: i32) -> Canvas {
    let theme = theme.scaled(s);
    let text = TextRenderer::load(&theme.fonts);
    let p = theme.paint();
    let f = form(calc, history);
    let (w, h) = client(calc.mode, history);
    let mut c = Canvas::new((w * s) as u32, (h * s) as u32);
    c.fill_rect(c.bounds(), p.cs.button_face);
    draw(&mut c, &text, &p, &f, calc, theme.fonts.size_px);
    c
}

#[test]
fn calculator_pins_at_scale_1_and_2() {
    if !crate::text::test_tahoma() {
        return;
    }
    let mut standard = after(Mode::Standard, "12+3*");
    standard.input(Input::MemStore);
    let scientific = after(Mode::Scientific, "2*(3+4\n");
    let error = after(Mode::Standard, "1/0=");
    let mut got = Vec::new();
    for (name, theme) in looks() {
        for (state, calc, history) in [
            ("standard", &standard, false),
            ("scientific with history", &scientific, true),
            ("error", &error, false),
        ] {
            for s in [1, 2] {
                got.push((
                    format!("{name} {state} {s}"),
                    fnv(&drawn(&theme, calc, history, s)),
                ));
            }
        }
    }
    crate::pins::check("calculator", &got);
}

#[test]
fn the_display_shows_the_number_and_an_m_with_memory() {
    if !crate::text::test_tahoma() {
        return;
    }
    let theme = Theme::default();
    let calc = after(Mode::Standard, "42");
    let mut stored = calc.clone();
    stored.input(Input::MemStore);
    let plain = drawn(&theme, &calc, false, 1);
    let marked = drawn(&theme, &stored, false, 1);
    assert_ne!(fnv(&plain), fnv(&marked), "the M shows");
    let wide = after(Mode::Standard, &"8".repeat(MAX_DIGITS));
    let f = form(&wide, false);
    let r = f.item(DISPLAY).expect("display").rect;
    let c = drawn(&theme, &wide, false, 1);
    let window = theme.colors.window.argb();
    let ink =
        (r.x + 3..r.x + 12).any(|x| (r.y + 20..r.bottom() - 4).any(|y| c.get(x, y) != window));
    assert!(!ink, "a long number shrinks to stay inside the display");
}
