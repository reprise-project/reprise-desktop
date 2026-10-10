//! The AppChooser's dialog (`reprise-portal --appchoose`): the Open With
//! chooser over the caller's programs, owned by and modal to the caller's
//! window. The first stdin document is the `Input`; each later one is an
//! `Update` that refills the list while it is up. The answer goes to
//! stdout as TOML.

use crate::appchooser::{Input, Output, Update};
use crate::dialog;
use repriseapp::openwith::ChooserInput;
use repriseapp::{App, Ctx, Event, Reply, Waker, WinId};
use reprisefs::apps::AppEntry;
use reprisekit::access::Tree;
use reprisekit::controls::openwith::{self, Check, OpenWith, Program};
use reprisekit::icontheme::IconCache;
use reprisekit::look::Paint;
use reprisekit::raster::Canvas;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};

/// `id` as a desktop-file name: the caller may send it with or without
/// its `.desktop`.
fn desktop_file(id: &str) -> String {
    if id.ends_with(".desktop") {
        id.to_string()
    } else {
        format!("{id}.desktop")
    }
}

/// The caller's ids as chooser rows: those whose desktop entry opens
/// `content_type` or is its `default` (a desktop-file id) are recommended,
/// the rest follow; each group by name. An id matches an entry with or
/// without its `.desktop`; one with no entry is listed by its id. Without a
/// type every choice is recommended.
pub fn group(
    choices: &[String],
    apps: &[AppEntry],
    content_type: Option<&str>,
    default: Option<&str>,
) -> (Vec<Program>, Vec<Program>) {
    let opens: Vec<usize> = content_type
        .map(|m| reprisefs::apps::open_with(apps, m))
        .unwrap_or_default();
    let (mut rec, mut oth) = (Vec::new(), Vec::new());
    for id in choices {
        let full = desktop_file(id);
        let at = apps.iter().position(|a| a.id == full);
        let p = match at {
            Some(i) => Program {
                id: id.clone(),
                name: apps[i].name.clone(),
                icon: apps[i].icon.clone(),
            },
            None => Program {
                id: id.clone(),
                name: id.clone(),
                icon: None,
            },
        };
        let recommended = content_type.is_none()
            || at.is_some_and(|i| opens.contains(&i))
            || default == Some(full.as_str());
        if recommended {
            rec.push(p)
        } else {
            oth.push(p)
        }
    }
    rec.sort_by_key(|p| p.name.to_lowercase());
    oth.sort_by_key(|p| p.name.to_lowercase());
    (rec, oth)
}

/// The control for `input`: Browse... greyed (the answer must be one of
/// the caller's ids), the box greyed without a type, `last_choice`
/// picked when it is offered, else the type's `default` when offered.
fn chooser(input: &Input, apps: &[AppEntry], default: Option<&str>) -> OpenWith {
    let (rec, oth) = group(&input.choices, apps, input.content_type.as_deref(), default);
    let mut ow = OpenWith::new(&input.file, rec, oth);
    if let Some(d) = default
        && let Some(id) = input.choices.iter().find(|c| desktop_file(c) == d)
    {
        ow.select_id(id);
    }
    ow.browse_enabled = false;
    ow.always = Check {
        on: false,
        enabled: input.content_type.is_some(),
    };
    if let Some(id) = &input.last_choice {
        ow.select_id(id);
    }
    ow
}

struct Choose {
    input: Input,
    apps: Vec<AppEntry>,
    ow: OpenWith,
    default: Option<String>,
    icon: Option<Canvas>,
    row_icons: Vec<Option<Canvas>>,
    icons: IconCache,
    /// Updates from the parent: the reader thread's end until `start`,
    /// then drained into `pending` by a forwarding thread.
    updates: Option<Receiver<String>>,
    pending: Arc<Mutex<Vec<String>>>,
    input_state: ChooserInput,
    out: Arc<Mutex<Output>>,
}

impl Choose {
    fn resolve_icons(&mut self) {
        self.row_icons = self
            .ow
            .list
            .rows
            .iter()
            .map(|r| openwith::icon_name(r).and_then(|n| self.icons.get(n, 16)))
            .collect();
    }

    fn apply_updates(&mut self) {
        let docs = std::mem::take(&mut *self.pending.lock().unwrap());
        for doc in docs {
            match toml::from_str::<Update>(&doc) {
                Ok(u) => {
                    let (rec, oth) = group(
                        &u.choices,
                        &self.apps,
                        self.input.content_type.as_deref(),
                        self.default.as_deref(),
                    );
                    self.input.choices = u.choices;
                    self.ow.set_programs(rec, oth);
                }
                Err(e) => log::warn!("appchoose update: {e}"),
            }
        }
        self.resolve_icons();
    }

    /// OK on `id`: with the box ticked it becomes the type's default
    /// program, then it is the answer.
    fn choose(&mut self, id: String, always: bool) -> Reply {
        if always && let Some(mime) = &self.input.content_type {
            let full = desktop_file(&id);
            if let Err(e) = reprisefs::props::set_default_app(mime, &full) {
                log::warn!("appchoose: cannot make {full} the default for {mime}: {e}");
            }
        }
        *self.out.lock().unwrap() = Output { choice: Some(id) };
        Self::quit()
    }

    fn quit() -> Reply {
        Reply {
            quit: true,
            ..Default::default()
        }
    }
}

impl App for Choose {
    fn title(&self) -> String {
        openwith::TITLE.into()
    }

    fn app_id(&self) -> String {
        "reprise.portal".into()
    }

    fn initial_size(&self) -> (i32, i32) {
        openwith::CLIENT
    }

    fn fixed(&self) -> bool {
        true
    }

    fn foreign_parent(&self) -> Option<String> {
        self.input.parent.clone()
    }

    fn modal(&self) -> bool {
        self.input.modal
    }

    /// The chooser has the dialog caption with the ? box, and no taskbar
    /// button even with no window to belong to.
    fn caption(&self, _win: WinId) -> repriseapp::Caption {
        repriseapp::Caption::Help
    }

    fn start(&mut self, waker: Waker) {
        let Some(rx) = self.updates.take() else {
            return;
        };
        let pending = self.pending.clone();
        std::thread::spawn(move || {
            for doc in rx {
                pending.lock().unwrap().push(doc);
                waker.wake();
            }
        });
    }

    /// The parent's updates, then the input as every chooser host reads
    /// it (`repriseapp::openwith`).
    fn event(&mut self, _win: WinId, ev: Event, ctx: &mut Ctx) -> Reply {
        let out = match ev {
            Event::CloseRequest => return Self::quit(),
            Event::Wake => {
                self.apply_updates();
                return Reply::redraw();
            }
            ev => {
                let lay = self.ow.layout(
                    &ctx.text,
                    ctx.theme.fonts.size_px,
                    (0, 0),
                    &ctx.theme.look.controls(),
                );
                self.input_state.event(&mut self.ow, &lay, &ev)
            }
        };
        match out {
            openwith::Event::None | openwith::Event::Browse => Reply::none(),
            openwith::Event::Redraw => Reply::redraw(),
            openwith::Event::Cancel => Self::quit(),
            openwith::Event::Choose { id, always } => self.choose(id, always),
        }
    }

    fn draw(&mut self, _win: WinId, c: &mut Canvas, ctx: &Ctx) {
        let px = ctx.theme.fonts.size_px;
        let lay = self
            .ow
            .layout(&ctx.text, px, (0, 0), &ctx.theme.look.controls());
        let rows: Vec<Option<&Canvas>> = self.row_icons.iter().map(Option::as_ref).collect();
        self.ow.draw(
            c,
            &ctx.text,
            &Paint {
                s: 1,
                ..ctx.theme.paint()
            },
            &lay,
            self.icon.as_ref(),
            &rows,
            self.input_state.cues(),
            px,
        );
    }

    fn access(&mut self, _win: WinId, tree: &mut Tree, ctx: &Ctx) {
        let lay = self.ow.layout(
            &ctx.text,
            ctx.theme.fonts.size_px,
            (0, 0),
            &ctx.theme.look.controls(),
        );
        self.ow.access(tree, &lay);
    }
}

/// The child's side: the chooser from stdin's first document, updates
/// from the rest, the answer on stdout.
pub fn run_dialog() {
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        for doc in dialog::docs(std::io::stdin().lock()) {
            if tx.send(doc).is_err() {
                break;
            }
        }
    });
    let Some(input) = rx.recv().ok().and_then(|d| {
        toml::from_str::<Input>(&d)
            .map_err(|e| log::warn!("appchoose input: {e}"))
            .ok()
    }) else {
        return;
    };
    let apps = reprisefs::apps::scan(&reprisefs::apps::default_dirs());
    let mut icons = dialog::icons();
    let icon = icons.get(openwith::ICON, dialog::ICON_PX);
    let default = input
        .content_type
        .as_deref()
        .and_then(|m| reprisefs::props::default_app(m, &apps))
        .map(|i| apps[i].id.clone());
    let ow = chooser(&input, &apps, default.as_deref());
    let out = Arc::new(Mutex::new(Output::default()));
    let mut app = Choose {
        input,
        apps,
        ow,
        default,
        icon,
        row_icons: Vec::new(),
        icons,
        updates: Some(rx),
        pending: Arc::new(Mutex::new(Vec::new())),
        input_state: ChooserInput::default(),
        out: out.clone(),
    };
    app.resolve_icons();
    repriseapp::run(Box::new(app));
    let out = out.lock().unwrap().clone();
    match toml::to_string(&out) {
        Ok(s) => print!("{s}"),
        Err(e) => log::warn!("appchoose answer: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reprisefs::apps::AppEntry;

    fn e(id: &str, name: &str, mimes: &[&str]) -> AppEntry {
        AppEntry {
            name: name.into(),
            exec_line: String::new(),
            terminal: false,
            id: id.into(),
            icon: Some(format!("{name}-icon")),
            startup_wm_class: None,
            categories: Vec::new(),
            mime_types: mimes.iter().map(|m| m.to_string()).collect(),
            shown: true,
        }
    }

    #[test]
    fn choices_that_open_the_type_are_recommended_and_the_rest_follow_by_name() {
        let apps = vec![
            e("vim.desktop", "Vim", &["text/plain"]),
            e("zed.desktop", "Zed", &[]),
            e("ark.desktop", "Ark", &[]),
        ];
        let choices = vec!["zed".into(), "vim".into(), "ark.desktop".into()];
        let (rec, oth) = group(&choices, &apps, Some("text/plain"), None);
        assert_eq!(
            rec.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
            ["vim"]
        );
        assert_eq!(
            oth.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
            ["ark.desktop", "zed"]
        );
        assert_eq!(rec[0].name, "Vim");
        assert_eq!(rec[0].icon.as_deref(), Some("Vim-icon"));
    }

    #[test]
    fn without_a_type_every_choice_is_recommended() {
        let apps = vec![e("vim.desktop", "Vim", &["text/plain"])];
        let (rec, oth) = group(&["vim".into()], &apps, None, None);
        assert_eq!(rec.len(), 1);
        assert!(oth.is_empty());
    }

    #[test]
    fn unknown_ids_are_listed_by_id_and_last_choice_must_be_offered() {
        let (rec, _) = group(&["ghost".into()], &[], None, None);
        assert_eq!(rec[0].name, "ghost");
        let mut ow = chooser(
            &Input {
                choices: vec!["ghost".into(), "other".into()],
                last_choice: Some("nope".into()),
                ..Default::default()
            },
            &[],
            None,
        );
        assert_eq!(
            ow.chosen().map(|p| p.id.as_str()),
            Some("ghost"),
            "an unoffered last_choice leaves the first pick"
        );
        ow = chooser(
            &Input {
                choices: vec!["ghost".into(), "other".into()],
                last_choice: Some("other".into()),
                ..Default::default()
            },
            &[],
            None,
        );
        assert_eq!(ow.chosen().map(|p| p.id.as_str()), Some("other"));
    }

    #[test]
    fn the_type_s_default_is_recommended_and_picked_unless_last_choice_is_offered() {
        let apps = vec![
            e("vim.desktop", "Vim", &["text/plain"]),
            e("zed.desktop", "Zed", &[]),
        ];
        let input = Input {
            content_type: Some("text/plain".into()),
            choices: vec!["vim".into(), "zed".into()],
            ..Default::default()
        };
        let ow = chooser(&input, &apps, Some("zed.desktop"));
        let names: Vec<&str> = ow.list.rows.iter().map(|r| r.label()).collect();
        assert_eq!(names, [openwith::RECOMMENDED, "Vim", "Zed"]);
        assert_eq!(ow.chosen().map(|p| p.id.as_str()), Some("zed"));
        let ow = chooser(
            &Input {
                last_choice: Some("vim".into()),
                ..input.clone()
            },
            &apps,
            Some("zed.desktop"),
        );
        assert_eq!(ow.chosen().map(|p| p.id.as_str()), Some("vim"));
        let ow = chooser(&input, &apps, Some("ark.desktop"));
        assert_eq!(
            ow.chosen().map(|p| p.id.as_str()),
            Some("vim"),
            "an unoffered default leaves the first pick"
        );
    }

    #[test]
    fn a_desktop_file_name_gets_its_suffix_once() {
        assert_eq!(desktop_file("vim"), "vim.desktop");
        assert_eq!(desktop_file("vim.desktop"), "vim.desktop");
    }

    #[test]
    fn an_update_regroups_the_list_and_keeps_the_pick() {
        let apps = vec![
            e("vim.desktop", "Vim", &["text/plain"]),
            e("zed.desktop", "Zed", &[]),
        ];
        let input = Input {
            content_type: Some("text/plain".into()),
            choices: vec!["zed".into()],
            last_choice: Some("zed".into()),
            ..Default::default()
        };
        let mut app = Choose {
            ow: chooser(&input, &apps, None),
            default: None,
            input,
            apps,
            icon: None,
            row_icons: Vec::new(),
            icons: IconCache::new(reprisekit::icontheme::IconTheme::load("hicolor", &[])),
            updates: None,
            pending: Arc::new(Mutex::new(Vec::new())),
            input_state: ChooserInput::default(),
            out: Arc::new(Mutex::new(Output::default())),
        };
        let update = Update {
            choices: vec!["vim".into(), "zed".into(), "ark".into()],
        };
        app.pending
            .lock()
            .unwrap()
            .push(toml::to_string(&update).unwrap());
        app.apply_updates();
        let names: Vec<&str> = app.ow.list.rows.iter().map(|r| r.label()).collect();
        assert_eq!(
            names,
            [openwith::RECOMMENDED, "Vim", openwith::OTHERS, "ark", "Zed"]
        );
        assert_eq!(app.ow.chosen().map(|p| p.id.as_str()), Some("zed"));
        assert_eq!(app.input.choices, update.choices);
        assert_eq!(app.row_icons.len(), 5, "one icon slot per row");
        assert!(
            app.pending.lock().unwrap().is_empty(),
            "the update is used up"
        );
    }

    #[test]
    fn the_box_greys_without_a_type_and_browse_is_always_greyed() {
        let ow = chooser(
            &Input {
                choices: vec!["a".into()],
                ..Default::default()
            },
            &[],
            None,
        );
        assert!(!ow.always.enabled);
        assert!(!ow.browse_enabled);
        let ow = chooser(
            &Input {
                choices: vec!["a".into()],
                content_type: Some("text/plain".into()),
                ..Default::default()
            },
            &[],
            None,
        );
        assert!(ow.always.enabled);
    }
}
