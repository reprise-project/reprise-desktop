//! The Window Rules page: `[[window_rules]]` in file order, each row a rule's program and title with
//! what it sets dimmed after them, and Add..., Edit..., Remove, Move Up and Move Down beside the
//! list. The list takes the page's extra height.

use crate::model::Label;
use crate::pages::{Area, X};
use crate::windowrules::Rule;
use reprisekit::classic::widgets::{BUTTON_H, BUTTON_W, LINE_H};
use reprisekit::controls::form::{Control, Form, Item};
use reprisekit::controls::listbox::{self, ListBox};
use reprisekit::raster::Rect;
use reprisekit::tr;

pub const LIST: &str = "window_rules";
pub const ADD: &str = "wr_add";
pub const EDIT: &str = "wr_edit";
pub const REMOVE: &str = "wr_remove";
pub const UP: &str = "wr_up";
pub const DOWN: &str = "wr_down";

/// The list's rows at [`Area::MIN`].
pub const ROWS: usize = 19;
const BUTTON_STEP: i32 = BUTTON_H + 4;
const LIST_Y: i32 = 78;

/// `[[window_rules]]` `state` values and the State combo's rows for them; "" leaves the key out.
pub const STATES: &[(&str, Label)] = &[
    ("", || tr!("rulespage-as-it-opens")),
    ("normal", || tr!("rulespage-normal")),
    ("maximized", || tr!("rulespage-maximized")),
    ("minimized", || tr!("rulespage-minimized")),
    ("fullscreen", || tr!("rulespage-full-screen")),
];

fn list_rect(a: Area) -> Rect {
    Rect::new(
        X,
        LIST_Y,
        a.w - BUTTON_W - 8,
        ROWS as i32 * listbox::ROW_H + 4 + a.dh,
    )
}

pub fn page(a: Area) -> Form {
    let list = list_rect(a);
    let mut items = vec![
        Item::new(
            "window_rules_note",
            Rect::new(X, 44, a.w, 2 * LINE_H),
            Control::Lines(vec![
                tr!("rulespage-a-window-takes-every-rule-matching"),
                tr!("rulespage-where-two-set-the-same-thing"),
            ]),
        ),
        Item::new(
            LIST,
            list,
            Control::ListBox {
                list: ListBox::default(),
                icons: Vec::new(),
            },
        )
        .named(&tr!("rulespage-window-rules")),
        Item::label_at(
            "window_rules_menu_note",
            X,
            list.bottom() + 8,
            a.w,
            &tr!("rulespage-always-open-here-on-a-windows"),
        ),
    ];
    let buttons = [
        (ADD, &tr!("rulespage-add")),
        (EDIT, &tr!("rulespage-edit")),
        (REMOVE, &tr!("rulespage-remove")),
        (UP, &tr!("rulespage-move-up")),
        (DOWN, &tr!("rulespage-move-down")),
    ];
    for (i, (id, text)) in buttons.into_iter().enumerate() {
        let r = Rect::new(
            a.right() - BUTTON_W,
            LIST_Y + i as i32 * BUTTON_STEP,
            BUTTON_W,
            BUTTON_H,
        );
        items.push(Item::button(id, r, text, true, false));
    }
    Form::new(items)
}

/// A row's text: the program, and the title when the rule matches one.
pub fn shown(r: &Rule) -> String {
    let app = match r.app_id.trim() {
        "" => &tr!("rulespage-no-app-id"),
        _ => &r.app_id,
    };
    match &r.title {
        Some(t) => tr!("rulespage-titled", app = app.as_str(), title = t.as_str()),
        None => app.to_string(),
    }
}

/// How Screen and the list name `output`: a connector as it is, a monitor id
/// (`displays::monitor_ids`) as its make and model with the serial in brackets.
pub fn screen(output: &str) -> String {
    let mut parts = output.split('|');
    let (Some(make), Some(model), Some(serial)) = (parts.next(), parts.next(), parts.next()) else {
        return output.to_string();
    };
    let serial = (!serial.is_empty()).then(|| tr!("rulespage-serial", serial = serial));
    let name = [Some(make.to_string()), Some(model.to_string()), serial]
        .into_iter()
        .flatten()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    match parts.next() {
        Some(connector) => tr!(
            "rulespage-name-on",
            name = name.as_str(),
            connector = connector
        ),
        None => name,
    }
}

/// What the rule sets, as the row's dimmed detail.
pub fn sets(r: &Rule) -> String {
    let on = |v: Option<bool>, yes: &str, no: &str| v.map(|v| if v { yes } else { no }.to_string());
    let parts: Vec<String> = [
        r.size
            .map(|[w, h]| tr!("rulespage-size", w = w.to_string(), h = h.to_string())),
        r.position
            .map(|[x, y]| tr!("rulespage-at", x = x.to_string(), y = y.to_string())),
        r.output
            .as_ref()
            .map(|o| tr!("rulespage-on", screen = screen(o))),
        (r.state.as_deref()).map(|s| {
            STATES
                .iter()
                .find(|(v, _)| *v == s)
                .map_or_else(|| s.to_string(), |(_, l)| l())
        }),
        r.desktop
            .map(|d| tr!("rulespage-desktop", d = d.to_string())),
        on(
            r.always_on_top,
            &tr!("rulespage-on-top"),
            &tr!("rulespage-not-on-top"),
        ),
        on(
            r.sticky,
            &tr!("rulespage-every-desktop"),
            &tr!("rulespage-one-desktop"),
        ),
        r.tab_group
            .as_ref()
            .map(|g| tr!("rulespage-tab-group", g = g.as_str())),
    ]
    .into_iter()
    .flatten()
    .collect();
    if parts.is_empty() {
        tr!("rulespage-sets-nothing")
    } else {
        parts.join(", ")
    }
}

/// The picked row.
pub fn picked(form: &Form) -> Option<usize> {
    match form.item(LIST).map(|i| &i.control) {
        Some(Control::ListBox { list, .. }) => list.selected,
        _ => None,
    }
}

/// Show `rules` with row `pick` (else the pick as it was) picked and in view, and grey the buttons
/// with nothing to act on; a greyed button the keyboard was on hands it to the list.
pub fn fill(form: &mut Form, rules: &[Rule], pick: Option<usize>) {
    let n = rules.len();
    if let Some(Control::ListBox { list, icons }) = form.item_mut(LIST).map(|i| &mut i.control) {
        let pick = pick.or(list.selected).map(|i| i.min(n.saturating_sub(1)));
        let top = list.top;
        *list = ListBox::new(rules.iter().map(shown).collect(), pick.filter(|_| n > 0));
        list.detail = rules.iter().map(sets).collect();
        list.top = match list.selected {
            Some(i) if i < top => i,
            Some(i) if i >= top + ROWS => i + 1 - ROWS,
            _ => top.min(n.saturating_sub(ROWS)),
        };
        *icons = vec![None; n];
    }
    let sel = picked(form);
    let lit = [
        (EDIT, sel.is_some()),
        (REMOVE, sel.is_some()),
        (UP, sel.is_some_and(|i| i > 0)),
        (DOWN, sel.is_some_and(|i| i + 1 < n)),
    ];
    for (id, on) in lit {
        form.set_enabled(id, on);
    }
    let focused = form.focus().and_then(|f| form.items.get(f)).map(|i| i.id);
    if lit.iter().any(|&(id, on)| !on && focused == Some(id)) {
        form.focus_id(LIST);
    }
}

/// Remove the picked rule; the pick moves to the one that took its place. False with nothing picked.
pub fn remove(form: &mut Form, rules: &mut Vec<Rule>) -> bool {
    let Some(i) = picked(form).filter(|&i| i < rules.len()) else {
        return false;
    };
    rules.remove(i);
    fill(form, rules, Some(i));
    true
}

/// Move the picked rule `by` rows, the pick with it; false at the list's end.
pub fn shift(form: &mut Form, rules: &mut [Rule], by: isize) -> bool {
    let Some(i) = picked(form).filter(|&i| i < rules.len()) else {
        return false;
    };
    let Some(j) = i.checked_add_signed(by).filter(|&j| j < rules.len()) else {
        return false;
    };
    rules.swap(i, j);
    fill(form, rules, Some(j));
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(app_id: &str) -> Rule {
        Rule {
            app_id: app_id.into(),
            ..Rule::default()
        }
    }

    fn lit(f: &Form, id: &str) -> bool {
        matches!(
            f.item(id).map(|i| &i.control),
            Some(Control::Button { enabled: true, .. })
        )
    }

    fn rows(f: &Form) -> Vec<String> {
        match f.item(LIST).map(|i| &i.control) {
            Some(Control::ListBox { list, .. }) => list.items.clone(),
            _ => panic!("the list is on the page"),
        }
    }

    #[test]
    fn a_row_shows_the_match_and_what_the_rule_sets() {
        let full = Rule {
            app_id: "foot".into(),
            title: Some("*build*".into()),
            size: Some([800, 600]),
            position: Some([40, -10]),
            output: Some("DP-1".into()),
            state: Some("fullscreen".into()),
            desktop: Some(2),
            always_on_top: Some(true),
            sticky: Some(false),
            tab_group: Some("shells".into()),
            from_menu: false,
        };
        assert_eq!(shown(&full), "foot, titled *build*");
        assert_eq!(
            sets(&full),
            "800 x 600, at 40, -10, on DP-1, Full screen, desktop 2, on top, one desktop, \
             tab group shells"
        );
        assert_eq!(
            (shown(&rule("foot")), sets(&rule("foot"))),
            ("foot".into(), "sets nothing".into())
        );
        assert_eq!(shown(&rule("")), "(no app ID)");
    }

    #[test]
    fn a_monitor_id_reads_as_the_monitor() {
        assert_eq!(screen("DP-1"), "DP-1");
        assert_eq!(
            screen("Dell Inc.|DELL U2723QE|ABC123"),
            "Dell Inc. DELL U2723QE (ABC123)"
        );
        assert_eq!(screen("|U2720Q|"), "U2720Q");
        assert_eq!(screen("||ABC123"), "(ABC123)");
        assert_eq!(screen("Dell|U2720Q||DP-2"), "Dell U2720Q on DP-2");
    }

    #[test]
    fn the_buttons_grey_with_nothing_to_act_on() {
        let mut f = page(Area::MIN);
        let mut rules = vec![rule("a"), rule("b"), rule("c")];
        fill(&mut f, &rules, None);
        assert_eq!(rows(&f), ["a", "b", "c"]);
        assert!(lit(&f, ADD));
        for id in [EDIT, REMOVE, UP, DOWN] {
            assert!(!lit(&f, id), "{id} greys with nothing picked");
        }
        fill(&mut f, &rules, Some(0));
        assert!(lit(&f, EDIT) && lit(&f, REMOVE) && !lit(&f, UP) && lit(&f, DOWN));
        assert!(shift(&mut f, &mut rules, 1));
        assert_eq!(rows(&f), ["b", "a", "c"]);
        assert_eq!(picked(&f), Some(1));
        assert!(shift(&mut f, &mut rules, 1));
        assert!(!shift(&mut f, &mut rules, 1), "the last row stays last");
        assert!(lit(&f, UP) && !lit(&f, DOWN));
        assert!(remove(&mut f, &mut rules));
        assert_eq!(
            (rows(&f), picked(&f)),
            (vec!["b".into(), "c".into()], Some(1))
        );
        assert!(remove(&mut f, &mut rules) && remove(&mut f, &mut rules));
        assert!(rows(&f).is_empty() && picked(&f).is_none());
        assert!(!remove(&mut f, &mut rules));
        for id in [EDIT, REMOVE, UP, DOWN] {
            assert!(!lit(&f, id), "{id} greys on an empty list");
        }
    }
}
