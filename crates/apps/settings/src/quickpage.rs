//! The Start Menu page's Quick Launch group: the folder's buttons in order,
//! with Add..., Remove, Move Up and Move Down. Each acts on the folder at
//! once, as the Startup page does; the compositor watches the folder.

use std::path::Path;

use reprisefs::apps;
use reprisefs::quicklaunch as ql;
use reprisekit::classic::widgets::{BUTTON_H, BUTTON_W};
use reprisekit::controls::form::{Control, Form, Item};
use reprisekit::controls::listbox::{self, ListBox};
use reprisekit::raster::{Canvas, Rect};

use crate::additem::NewItem;
use crate::model::ACTIONS;
use crate::pages::{Area, GX, X};

pub const LIST: &str = "ql_list";
pub const ADD: &str = "ql_add";
pub const REMOVE: &str = "ql_remove";
pub const UP: &str = "ql_up";
pub const DOWN: &str = "ql_down";

/// The group, under the Start menu group; it moves down with the page's extra height.
fn group_rect(a: Area) -> Rect {
    Rect::new(X, 248 + a.dh, a.w, 134)
}
pub fn list_rect(a: Area) -> Rect {
    Rect::new(
        GX,
        268 + a.dh,
        a.gw() - BUTTON_W - 8,
        6 * listbox::ROW_H + 4,
    )
}
const BUTTON_STEP: i32 = BUTTON_H + 4;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Launcher {
    pub file: String,
    pub name: String,
    pub icon: Option<String>,
}

/// The group's items in `a`, for the Start Menu page to carry under its own.
pub fn items(a: Area) -> Vec<Item> {
    let list_rect = list_rect(a);
    let mut items = vec![
        Item::group("ql_group", group_rect(a), "Quick Launch"),
        Item::new(
            LIST,
            list_rect,
            Control::ListBox {
                list: ListBox::default(),
                icons: Vec::new(),
            },
        ),
    ];
    let buttons = [
        (ADD, "A&dd..."),
        (REMOVE, "Remo&ve"),
        (UP, "Move U&p"),
        (DOWN, "Move D&own"),
    ];
    for (i, (id, text)) in buttons.into_iter().enumerate() {
        let r = Rect::new(
            a.gright() - BUTTON_W,
            list_rect.y + i as i32 * BUTTON_STEP,
            BUTTON_W,
            BUTTON_H,
        );
        items.push(Item::button(id, r, text, true, false));
    }
    items
}

/// The folder's buttons in taskbar order: files the taskbar takes (a program's entry or a Reprise action).
pub fn load(dir: &Path) -> Vec<Launcher> {
    let mut v: Vec<Launcher> = ql::files(dir)
        .into_iter()
        .filter_map(|file| {
            let text = apps::read_desktop_file(&dir.join(&file)).ok()?;
            // `spawn` needs a command, which a button file does not carry.
            let is_action = |a: &str| a != "spawn" && ACTIONS.contains(&a);
            let (name, icon) = match ql::button(&text, &file, is_action)? {
                ql::Button::App(app) => (app.name, app.icon),
                ql::Button::Action { name, icon, .. } => (name, icon),
            };
            Some(Launcher { file, name, icon })
        })
        .collect();
    if let Some(order) = ql::read_order(dir) {
        ql::apply_order(&mut v, &order, |l| &l.file);
    }
    v
}

pub fn store(dir: &Path, list: &[Launcher]) -> std::io::Result<()> {
    let files: Vec<String> = list.iter().map(|l| l.file.clone()).collect();
    ql::write_order(dir, &files)
}

/// Write a button for `item` into `dir` and return its file. A picked program's
/// `.desktop` is copied under its own name (renamed if changed) so the taskbar
/// matches its windows; a typed command becomes a launcher of its own.
pub fn add(dir: &Path, item: &NewItem) -> std::io::Result<String> {
    std::fs::create_dir_all(dir)?;
    let copied = item.app.as_ref().and_then(|a| {
        let text = apps::read_desktop_file(&apps::find_file(&a.id)?).ok()?;
        let text = if item.name == a.name {
            text
        } else {
            apps::with_name(&text, &item.name)
        };
        Some((ql::unique_name(dir, &a.id), text))
    });
    let (file, text) = match copied {
        Some(c) => c,
        None => {
            let text = apps::launcher_text(&item.name, None, apps::Launch::Exec(&item.command))
                .ok_or_else(|| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "the command holds a control character",
                    )
                })?;
            let name = format!("{}.desktop", ql::slug(&item.name));
            (ql::unique_name(dir, &name), text)
        }
    };
    std::fs::write(dir.join(&file), text)?;
    Ok(file)
}

pub fn remove(dir: &Path, file: &str) -> std::io::Result<()> {
    std::fs::remove_file(dir.join(file))
}

fn list_mut(form: &mut Form) -> Option<(&mut ListBox, &mut Vec<Option<Canvas>>)> {
    match form.item_mut(LIST).map(|i| &mut i.control) {
        Some(Control::ListBox { list, icons }) => Some((list, icons)),
        _ => None,
    }
}

pub fn selected(form: &Form) -> Option<usize> {
    match form.item(LIST).map(|i| &i.control) {
        Some(Control::ListBox { list, .. }) => list.selected,
        _ => None,
    }
}

/// Show `list` with row `pick` scrolled into view, then grey buttons with nothing to act on.
pub fn fill(form: &mut Form, list: &[Launcher], pick: Option<usize>) {
    let visible = ((list_rect(Area::MIN).height - 4) / listbox::ROW_H) as usize;

    if let Some((lb, icons)) = list_mut(form) {
        let rows = list.iter().map(|l| l.name.clone()).collect();
        *lb = ListBox::new(rows, pick.filter(|&i| i < list.len()));
        if let Some(i) = lb.selected {
            lb.top = i.saturating_sub(visible - 1);
        }
        icons.clear();
    }
    sync(form, list.len());
}

pub fn sync(form: &mut Form, n: usize) {
    let sel = selected(form).filter(|&i| i < n);
    form.set_enabled(REMOVE, sel.is_some());
    form.set_enabled(UP, sel.is_some_and(|i| i > 0));
    form.set_enabled(DOWN, sel.is_some_and(|i| i + 1 < n));
}

pub fn set_icons(form: &mut Form, list: &[Launcher], get: &mut dyn FnMut(&str) -> Option<Canvas>) {
    let got = list
        .iter()
        .map(|l| get(l.icon.as_deref().unwrap_or("application-x-executable")))
        .collect();
    if let Some((_, icons)) = list_mut(form) {
        *icons = got;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmp(tag: &str) -> PathBuf {
        let p =
            std::env::temp_dir().join(format!("reprise-quickpage-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn launcher(dir: &Path, file: &str, name: &str) {
        let text = apps::launcher_text(name, Some("x"), apps::Launch::Exec("true")).unwrap();
        std::fs::write(dir.join(file), text).unwrap();
    }

    fn names(v: &[Launcher]) -> Vec<&str> {
        v.iter().map(|l| l.name.as_str()).collect()
    }

    #[test]
    fn the_folder_lists_in_its_order_file_then_by_name() {
        let dir = tmp("order");
        launcher(&dir, "a.desktop", "A");
        launcher(&dir, "b.desktop", "B");
        launcher(&dir, "c.desktop", "C");
        std::fs::write(dir.join("nameless.desktop"), "[Desktop Entry]\n").unwrap();
        let action =
            "[Desktop Entry]\nType=Application\nName=Show Desktop\nX-Reprise-Action=showdesktop\n";
        std::fs::write(dir.join("d.desktop"), action).unwrap();
        let link = "[Desktop Entry]\nType=Link\nName=Site\nURL=https://example.org\n";
        std::fs::write(dir.join("link.desktop"), link).unwrap();
        assert_eq!(names(&load(&dir)), ["A", "B", "C", "Show Desktop"]);
        remove(&dir, "d.desktop").unwrap();
        ql::write_order(&dir, &["c.desktop".into(), "a.desktop".into()]).unwrap();
        let mut v = load(&dir);
        assert_eq!(names(&v), ["C", "A", "B"]);
        v.swap(1, 2);
        store(&dir, &v).unwrap();
        assert_eq!(names(&load(&dir)), ["C", "B", "A"]);
        remove(&dir, "b.desktop").unwrap();
        assert_eq!(names(&load(&dir)), ["C", "A"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A file naming an action the compositor has no button for is left out, as the taskbar does.
    #[test]
    fn a_file_with_an_unknown_action_is_no_button() {
        let dir = tmp("bogus");
        let file = |action: &str| {
            format!("[Desktop Entry]\nType=Application\nName={action}\nX-Reprise-Action={action}\n")
        };
        std::fs::write(dir.join("a.desktop"), file("showdesktop")).unwrap();
        std::fs::write(dir.join("b.desktop"), file("bogus")).unwrap();
        std::fs::write(dir.join("c.desktop"), file("spawn")).unwrap();
        assert_eq!(names(&load(&dir)), ["showdesktop"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_typed_command_becomes_a_launcher_named_after_it() {
        let dir = tmp("typed");
        let item = NewItem {
            name: "My Files".into(),
            command: "nautilus --new-window".into(),
            icon: None,
            app: None,
        };
        assert_eq!(add(&dir, &item).unwrap(), "my-files.desktop");
        assert_eq!(add(&dir, &item).unwrap(), "my-files (2).desktop");
        let text = std::fs::read_to_string(dir.join("my-files.desktop")).unwrap();
        assert!(text.contains("Name=My Files\n") && text.contains("Exec=nautilus --new-window\n"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_buttons_follow_the_picked_row() {
        let mut f = Form::new(items(Area::MIN));
        let list = |n: usize| -> Vec<Launcher> {
            (0..n)
                .map(|i| Launcher {
                    file: format!("{i}.desktop"),
                    name: format!("{i}"),
                    icon: None,
                })
                .collect()
        };
        let on = |f: &Form, id| match f.item(id).map(|i| &i.control) {
            Some(Control::Button { enabled, .. }) => *enabled,
            _ => panic!("{id} is a button"),
        };
        fill(&mut f, &list(3), None);
        assert!(on(&f, ADD) && !on(&f, REMOVE) && !on(&f, UP) && !on(&f, DOWN));
        fill(&mut f, &list(3), Some(0));
        assert!(on(&f, REMOVE) && !on(&f, UP) && on(&f, DOWN));
        fill(&mut f, &list(3), Some(2));
        assert!(on(&f, UP) && !on(&f, DOWN));
        fill(&mut f, &list(8), Some(7));
        match f.item(LIST).map(|i| &i.control) {
            Some(Control::ListBox { list, .. }) => assert_eq!(list.top, 2, "the last row shows"),
            _ => panic!("a list box"),
        }
    }

    #[test]
    fn the_group_sits_under_the_start_menus_with_its_buttons_beside_the_list() {
        let f = Form::new(items(Area::MIN));
        assert_eq!(list_rect(Area::MIN), Rect::new(32, 268, 251, 106));
        let tops: Vec<i32> = [ADD, REMOVE, UP, DOWN]
            .iter()
            .map(|id| f.item(id).unwrap().rect.y)
            .collect();
        assert_eq!(tops, [268, 295, 322, 349]);
        assert!(group_rect(Area::MIN).bottom() >= list_rect(Area::MIN).bottom() + 8);
        assert!(group_rect(Area::MIN).y >= crate::menupage::tree_rect(Area::MIN).bottom() + 20);
    }
}
