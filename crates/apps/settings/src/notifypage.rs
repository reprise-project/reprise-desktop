//! The Sounds page's program list: this session's senders and the programs `[notifications]
//! no_balloons` and `no_sounds` name, with Show balloons and Play sounds for the picked one.

use crate::model::Settings;
use repriseconf::config::{program_listed, set_program_listed};
use reprisekit::controls::form::{Control, Form};
use reprisekit::controls::listbox::ListBox;

pub const PROGRAMS: &str = "notify_programs";
pub const BALLOONS: &str = "notify_balloons";
pub const SOUNDS: &str = "notify_sounds";

/// The list's rows at [`Area::MIN`](crate::pages::Area::MIN).
pub const ROWS: usize = 5;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Program {
    /// What the config lists: the desktop entry id, else the app name.
    pub key: String,
    pub name: String,
}

/// The rows: `seen` (key, name) in the order they came, then each key `s` lists that is not among
/// them, named by `name_of` or else by its key.
pub fn programs(
    seen: &[(String, String)],
    s: &Settings,
    name_of: impl Fn(&str) -> Option<String>,
) -> Vec<Program> {
    let mut out: Vec<Program> = Vec::new();
    let configured = s.notify_no_balloons.iter().chain(&s.notify_no_sounds);
    let seen = seen.iter().map(|(k, n)| (k.clone(), Some(n.clone())));
    for (key, name) in seen.chain(configured.map(|k| (k.clone(), None))) {
        if key.is_empty() || out.iter().any(|p| p.key.eq_ignore_ascii_case(&key)) {
            continue;
        }
        let name = name
            .filter(|n| !n.is_empty())
            .or_else(|| name_of(&key))
            .unwrap_or_else(|| key.clone());
        out.push(Program { key, name });
    }
    out
}

fn detail(s: &Settings, key: &str) -> &'static str {
    let balloons = !program_listed(&s.notify_no_balloons, key);
    let sounds = !program_listed(&s.notify_no_sounds, key);
    match (balloons, sounds) {
        (true, true) => "",
        (false, true) => "No balloons",
        (true, false) => "No sounds",
        (false, false) => "No balloons or sounds",
    }
}

fn list(form: &Form) -> Option<&ListBox> {
    match form.item(PROGRAMS).map(|i| &i.control) {
        Some(Control::ListBox { list, .. }) => Some(list),
        _ => None,
    }
}

/// The program picked in the list.
pub fn picked<'a>(form: &Form, programs: &'a [Program]) -> Option<&'a Program> {
    programs.get(list(form)?.selected?)
}

/// Show `programs` under `s`: the pick stays on its program, each row says what is off, and the check
/// boxes show the picked program's switches, greyed while none is picked.
pub fn fill(form: &mut Form, programs: &[Program], s: &Settings) {
    let Some(Control::ListBox { list, icons }) = form.item_mut(PROGRAMS).map(|i| &mut i.control)
    else {
        return;
    };
    let names: Vec<String> = programs.iter().map(|p| p.name.clone()).collect();
    if list.items != names {
        let picked = list.selected.and_then(|i| list.items.get(i)).cloned();
        let top = list.top;
        *list = ListBox::new(names, None);
        list.selected = picked.and_then(|n| list.items.iter().position(|i| *i == n));
        list.top = top.min(list.items.len().saturating_sub(ROWS));
        *icons = vec![None; programs.len()];
    }
    list.detail = programs.iter().map(|p| detail(s, &p.key).into()).collect();
    let picked = list.selected.and_then(|i| programs.get(i));
    let on = picked.map(|p| {
        (
            !program_listed(&s.notify_no_balloons, &p.key),
            !program_listed(&s.notify_no_sounds, &p.key),
        )
    });
    for (id, checked) in [(BALLOONS, on.map(|o| o.0)), (SOUNDS, on.map(|o| o.1))] {
        form.set_enabled(id, checked.is_some());
        form.set_check(id, checked.unwrap_or(false));
    }
}

/// Check box `id` moved: its state goes into `s` for the picked program. True when `s` changed.
pub fn read(form: &Form, programs: &[Program], id: &str, s: &mut Settings) -> bool {
    let Some(p) = picked(form, programs) else {
        return false;
    };
    let list = match id {
        BALLOONS => &mut s.notify_no_balloons,
        SOUNDS => &mut s.notify_no_sounds,
        _ => return false,
    };
    set_program_listed(list, &p.key, form.checked(id) == Some(false))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pages::{Area, sounds};

    fn seen() -> Vec<(String, String)> {
        vec![
            ("mail".into(), "Mail".into()),
            ("chat".into(), "Chat".into()),
        ]
    }

    fn lit(form: &Form, id: &str) -> bool {
        matches!(
            form.item(id).map(|i| &i.control),
            Some(Control::Check { enabled: true, .. })
        )
    }

    fn pick(form: &mut Form, i: usize) {
        if let Some(Control::ListBox { list, .. }) = form.item_mut(PROGRAMS).map(|i| &mut i.control)
        {
            list.selected = Some(i);
        }
    }

    #[test]
    fn the_list_holds_the_session_programs_then_the_configured_ones() {
        let s = Settings {
            notify_no_balloons: vec!["Chat".into(), "org.example.Mail".into()],
            notify_no_sounds: vec!["gone".into()],
            ..Settings::default()
        };
        let named = |k: &str| (k == "org.example.Mail").then(|| "Example Mail".to_string());
        let got: Vec<(String, String)> = programs(&seen(), &s, named)
            .into_iter()
            .map(|p| (p.key, p.name))
            .collect();
        let want = [
            ("mail", "Mail"),
            ("chat", "Chat"),
            ("org.example.Mail", "Example Mail"),
            ("gone", "gone"),
        ]
        .map(|(k, n)| (k.to_string(), n.to_string()));
        assert_eq!(got, want);
    }

    #[test]
    fn the_check_boxes_follow_the_picked_program_and_write_its_switches() {
        let mut s = Settings {
            notify_no_sounds: vec!["chat".into()],
            ..Settings::default()
        };
        let mut f = sounds(Area::MIN, &[]);
        let list = programs(&seen(), &s, |_| None);
        fill(&mut f, &list, &s);
        assert!(
            !lit(&f, BALLOONS) && !lit(&f, SOUNDS),
            "greyed with nothing picked"
        );
        let Some(Control::ListBox { list: lb, .. }) = f.item(PROGRAMS).map(|i| &i.control) else {
            panic!("the list is on the page");
        };
        assert_eq!(lb.items, ["Mail", "Chat"]);
        assert_eq!(lb.detail, ["", "No sounds"]);

        pick(&mut f, 1);
        fill(&mut f, &list, &s);
        assert!(lit(&f, BALLOONS) && lit(&f, SOUNDS));
        assert_eq!(
            (f.checked(BALLOONS), f.checked(SOUNDS)),
            (Some(true), Some(false))
        );

        f.set_check(BALLOONS, false);
        assert!(read(&f, &list, BALLOONS, &mut s));
        assert_eq!(s.notify_no_balloons, ["chat"]);
        f.set_check(SOUNDS, true);
        assert!(read(&f, &list, SOUNDS, &mut s));
        assert!(s.notify_no_sounds.is_empty());
        assert!(!read(&f, &list, SOUNDS, &mut s), "nothing moved");
    }

    #[test]
    fn a_refill_keeps_the_pick_on_its_program() {
        let s = Settings::default();
        let mut f = sounds(Area::MIN, &[]);
        fill(&mut f, &programs(&seen(), &s, |_| None), &s);
        pick(&mut f, 1);
        let more = [vec![("new".to_string(), "New".to_string())], seen()].concat();
        let list = programs(&more, &s, |_| None);
        fill(&mut f, &list, &s);
        assert_eq!(picked(&f, &list).map(|p| p.key.as_str()), Some("chat"));
    }
}
