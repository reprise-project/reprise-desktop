//! The Default Programs page: a combo per kind of program, the terminal's arguments, what a plugged-in drive does (`[drives] on_insert`),
//! whether opening a script runs it bare or in the terminal (`script_open`), whether a script's
//! terminal stays open when it ends (`script_terminal`), whether a script asks before its first
//! run (`confirm_scripts`),
//! every file type with its program, a filter, and Change... and Restore Default. Defaults live in the mime-apps lists ([`reprisefs::props`]); edits wait in
//! [`Defaults::edits`] until Apply and go to the user's `mimeapps.list`.

use std::path::Path;

use reprisefs::apps::AppEntry;
use reprisefs::mimetree::MimeTree;
use reprisefs::props;
use reprisekit::classic::widgets::{BUTTON_GAP, BUTTON_H, BUTTON_W};
use reprisekit::controls::combo::Combo;
use reprisekit::controls::form::{Control, Form, Item};
use reprisekit::controls::listview::{Column, ListView, Row};
use reprisekit::raster::Rect;

use crate::pages::{Area, X, button};

/// One kind of program: its combo and label, and the types it is the
/// default for. The combo shows the default of `types[0]` and offers the
/// programs that declare it; a pick becomes the default of the first
/// `always` types, and of the rest that the program declares.
pub struct Kind {
    pub id: &'static str,
    pub label_id: &'static str,
    pub label: &'static str,
    pub types: &'static [&'static str],
    pub always: usize,
}

pub const KINDS: [Kind; 7] = [
    Kind {
        id: "dp_browser",
        label_id: "dp_browser_label",
        label: "&Web browser:",
        types: &[
            "x-scheme-handler/https",
            "x-scheme-handler/http",
            "text/html",
            "application/xhtml+xml",
        ],
        always: 3,
    },
    Kind {
        id: "dp_mail",
        label_id: "dp_mail_label",
        label: "&Mail:",
        types: &["x-scheme-handler/mailto"],
        always: 1,
    },
    Kind {
        id: "dp_music",
        label_id: "dp_music_label",
        label: "M&usic:",
        types: &[
            "audio/mpeg",
            "audio/flac",
            "audio/x-flac",
            "audio/ogg",
            "audio/x-vorbis+ogg",
            "audio/x-opus+ogg",
            "audio/mp4",
            "audio/aac",
            "audio/x-wav",
        ],
        always: 1,
    },
    Kind {
        id: "dp_video",
        label_id: "dp_video_label",
        label: "&Video:",
        types: &[
            "video/mp4",
            "video/x-matroska",
            "video/webm",
            "video/mpeg",
            "video/quicktime",
            "video/x-msvideo",
            "video/ogg",
        ],
        always: 1,
    },
    Kind {
        id: "dp_photos",
        label_id: "dp_photos_label",
        label: "P&hotos:",
        types: &[
            "image/jpeg",
            "image/png",
            "image/gif",
            "image/webp",
            "image/bmp",
            "image/tiff",
        ],
        always: 1,
    },
    Kind {
        id: "dp_editor",
        label_id: "dp_editor_label",
        label: "Te&xt editor:",
        types: &["text/plain"],
        always: 1,
    },
    Kind {
        id: "dp_files",
        label_id: "dp_files_label",
        label: "&File manager:",
        types: &["inode/directory"],
        always: 1,
    },
];

pub const TERMINAL: &str = "dp_terminal";
const TERMINAL_LABEL: &str = "dp_terminal_label";
pub const TERMINAL_ARGS: &str = "dp_terminal_args";
const TERMINAL_ARGS_LABEL: &str = "dp_terminal_args_label";
pub const DRIVE_INSERT: &str = "dp_drive_insert";
const DRIVE_INSERT_LABEL: &str = "dp_drive_insert_label";
pub const SCRIPT_OPEN: &str = "dp_script_open";
const SCRIPT_OPEN_LABEL: &str = "dp_script_open_label";
pub const SCRIPT_END: &str = "dp_script_end";
const SCRIPT_END_LABEL: &str = "dp_script_end_label";
pub const CONFIRM_SCRIPTS: &str = "dp_confirm_scripts";
const DRIVE_LABEL_W: i32 = 140;
pub const FILTER: &str = "dp_filter";
pub const CHANGE: &str = "dp_change";
pub const RESTORE: &str = "dp_restore";

pub const NONE_ROW: &str = "(None)";

const ROW0: i32 = 48;
const ROW_STEP: i32 = 23;
const LABEL_W: i32 = 96;
const DROP_ROWS: usize = 12;

pub fn list(a: Area) -> Rect {
    Rect::new(X, 381, a.w, 59 + a.dh)
}
pub const COLUMNS: [(&str, i32); 3] = [("Type", 90), ("Description", 140), ("Opens With", 108)];
const RESTORE_W: i32 = 96;

/// The page in `a` as built, before [`fill`] puts the programs in. The
/// combos grow with the column up to their most.
pub fn page(a: Area) -> Form {
    let list = list(a);
    let mut items = Vec::new();
    let labels = KINDS.iter().map(|k| (k.label_id, k.label, k.id)).chain([(
        TERMINAL_LABEL,
        "&Terminal:",
        TERMINAL,
    )]);
    for (i, (label_id, label, id)) in labels.enumerate() {
        let y = ROW0 + i as i32 * ROW_STEP;
        items.push(Item::label_at(label_id, X, y, LABEL_W, label));
        items.push(Item::combo(
            id,
            X + LABEL_W + 4,
            y - 4,
            a.combo(Area::MIN.w - LABEL_W - 4),
            &[],
            DROP_ROWS,
        ));
    }
    let y = ROW0 + (KINDS.len() as i32 + 1) * ROW_STEP;
    items.push(Item::label_at(
        TERMINAL_ARGS_LABEL,
        X,
        y,
        DRIVE_LABEL_W,
        "Terminal ar&guments:",
    ));
    items.push(Item::edit(
        TERMINAL_ARGS,
        Rect::new(
            X + DRIVE_LABEL_W + 4,
            y - 4,
            a.combo(Area::MIN.w - DRIVE_LABEL_W - 4),
            20,
        ),
        "",
        true,
    ));
    let y = y + ROW_STEP;
    items.push(Item::label_at(
        DRIVE_INSERT_LABEL,
        X,
        y,
        DRIVE_LABEL_W,
        "When a &drive is plugged in:",
    ));
    items.push(crate::pages::choice(
        DRIVE_INSERT,
        X + DRIVE_LABEL_W + 4,
        y - 4,
        a.combo(Area::MIN.w - DRIVE_LABEL_W - 4),
        crate::model::DRIVE_INSERTS,
    ));
    let y = y + ROW_STEP;
    items.push(Item::label_at(
        SCRIPT_OPEN_LABEL,
        X,
        y,
        DRIVE_LABEL_W,
        "When a script is &opened:",
    ));
    items.push(crate::pages::choice(
        SCRIPT_OPEN,
        X + DRIVE_LABEL_W + 4,
        y - 4,
        a.combo(Area::MIN.w - DRIVE_LABEL_W - 4),
        crate::model::SCRIPT_OPENS,
    ));
    let y = y + ROW_STEP;
    items.push(Item::label_at(
        SCRIPT_END_LABEL,
        X,
        y,
        DRIVE_LABEL_W,
        "When a &script ends:",
    ));
    items.push(crate::pages::choice(
        SCRIPT_END,
        X + DRIVE_LABEL_W + 4,
        y - 4,
        a.combo(Area::MIN.w - DRIVE_LABEL_W - 4),
        crate::model::SCRIPT_ENDS,
    ));
    items.push(Item::check(
        CONFIRM_SCRIPTS,
        X,
        y + ROW_STEP,
        "As&k before running a script the first time",
    ));
    let y = list.y - 22;
    items.push(Item::label_at("dp_types_label", X, y, 120, "File types:"));
    let fw = a.combo(150);
    items.push(Item::label_at(
        "dp_filter_label",
        a.right() - fw - 40,
        y,
        36,
        "Fi&lter:",
    ));
    items.push(Item::edit(
        FILTER,
        Rect::new(a.right() - fw, y - 4, fw, 20),
        "",
        true,
    ));
    let by = list.bottom() + 6;
    items.push(button(
        CHANGE,
        list.right() - RESTORE_W - BUTTON_GAP - BUTTON_W,
        by,
        "&Change...",
    ));
    items.push(Item::button(
        RESTORE,
        Rect::new(list.right() - RESTORE_W, by, RESTORE_W, BUTTON_H),
        "&Restore Default",
        true,
        false,
    ));
    Form::new(items)
}

pub fn new_list() -> ListView {
    ListView {
        columns: COLUMNS
            .iter()
            .map(|(title, w)| Column::new(title, *w))
            .collect(),
        ..ListView::default()
    }
}

pub fn fit_columns(
    lv: &mut ListView,
    cm: &reprisekit::controls::metrics::ControlMetrics,
    rect: Rect,
) {
    lv.fill_beside_bar(2, rect, cm);
}

/// A terminal the Terminal combo offers: its name, and the command the
/// `terminal` key gets (the program its entry runs).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Terminal {
    pub name: String,
    pub command: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileType {
    pub mime: String,
    pub ext: Option<String>,
    pub description: String,
}

pub struct Defaults {
    pub apps: Vec<AppEntry>,
    tree: MimeTree,
    /// Every mime-apps list in lookup order, the user's as it will be
    /// once `edits` are written.
    lists: Vec<String>,
    user: usize,
    loaded: String,
    /// The changes waiting for Apply, in order, one per type: a program
    /// id, or None to drop the user's choice.
    pub edits: Vec<(String, Option<String>)>,
    pub types: Vec<FileType>,
    pub terminals: Vec<Terminal>,
    filter: String,
    pub shown: Vec<usize>,
}

impl Defaults {
    /// The page over the installed programs and the lists as they are
    /// on disk.
    pub fn load() -> Defaults {
        let apps = reprisefs::apps::scan(&reprisefs::apps::default_dirs());
        let paths = props::mimeapps_paths();
        let user_path = props::user_mimeapps().ok();
        let user = user_path
            .as_ref()
            .and_then(|u| paths.iter().position(|p| p == u))
            .unwrap_or(0);
        let lists = paths
            .iter()
            .map(|p| std::fs::read_to_string(p).unwrap_or_default())
            .collect();
        let globs = reprisefs::apps::mime_globs();
        Defaults::new(
            apps,
            MimeTree::load(),
            lists,
            user,
            &globs.extensions(),
            &|m| globs.description(m),
        )
    }

    /// The page over `apps` and `lists` (in lookup order, the user's at
    /// `user`): `ext` and `describe` name each type.
    pub fn new(
        apps: Vec<AppEntry>,
        tree: MimeTree,
        lists: Vec<String>,
        user: usize,
        ext: &std::collections::HashMap<String, String>,
        describe: &dyn Fn(&str) -> Option<String>,
    ) -> Defaults {
        let types = file_types(&apps, &lists, ext, describe);
        let terminals = terminals(&apps);
        let mut d = Defaults {
            loaded: lists.get(user).cloned().unwrap_or_default(),
            apps,
            tree,
            lists,
            user,
            edits: Vec::new(),
            types,
            terminals,
            filter: String::new(),
            shown: Vec::new(),
        };
        d.refilter();
        d
    }

    pub fn current(&self, mime: &str) -> Option<usize> {
        props::default_in_lists(&self.lists, &self.tree, mime, &self.apps)
    }

    /// The program id for chooser row `id`. A program file Browse... picked
    /// (its path) gets a desktop entry for `mime` in applications folder
    /// `dir` (`None`: no home) and joins the programs.
    pub fn chosen(&mut self, id: &str, mime: &str, dir: Option<&Path>) -> std::io::Result<String> {
        if !Path::new(id).is_absolute() {
            return Ok(id.to_string());
        }
        let dir = dir.ok_or_else(|| std::io::Error::other("no home directory"))?;
        let entry = reprisefs::apps::program_entry_in(dir, Path::new(id), Some(mime))?;
        let id = entry.id.clone();
        match self.apps.iter_mut().find(|a| a.id == id) {
            Some(a) => *a = entry,
            None => self.apps.push(entry),
        }
        Ok(id)
    }

    /// Whether the user's list names a program for `mime`, edits
    /// included: what Restore Default takes away.
    pub fn user_set(&self, mime: &str) -> bool {
        self.lists
            .get(self.user)
            .is_some_and(|t| props::default_in_list(t, mime).is_some())
    }

    /// Make `id` the default for `mime`, or drop the user's choice with
    /// None; it waits for Apply.
    pub fn set(&mut self, mime: &str, id: Option<&str>) {
        self.edits.retain(|(m, _)| m != mime);
        self.edits.push((mime.to_string(), id.map(String::from)));
        self.rebuild_user();
    }

    fn rebuild_user(&mut self) {
        let text = props::with_edits(&self.loaded, &self.edits);
        match self.lists.get_mut(self.user) {
            Some(t) => *t = text,
            None => self.lists.push(text),
        }
    }

    pub fn dirty(&self) -> bool {
        self.lists.get(self.user).map_or("", String::as_str) != self.loaded
    }

    pub fn applied(&mut self, now: String) {
        self.loaded = now;
        self.edits.clear();
        self.rebuild_user();
    }

    pub fn revert(&mut self) {
        self.edits.clear();
        self.rebuild_user();
    }

    /// The programs kind `k`'s combo offers, by name: those that declare
    /// its first type, and its default if that is not one of them. With
    /// the default's place among them.
    pub fn offer(&self, k: &Kind) -> (Vec<usize>, Option<usize>) {
        let mime = k.types[0];
        let current = self.current(mime);
        let mut offer: Vec<usize> = (0..self.apps.len())
            .filter(|&i| {
                let a = &self.apps[i];
                (a.shown || Some(i) == current) && a.mime_types.iter().any(|m| m == mime)
            })
            .chain(current)
            .collect();
        offer.sort_by_key(|&i| (self.apps[i].name.to_lowercase(), i));
        offer.dedup();
        let at = current.and_then(|c| offer.iter().position(|&i| i == c));
        (offer, at)
    }

    /// The combo's rows for kind `k` and the picked one: "(None)" first
    /// while its type has no program.
    pub fn kind_rows(&self, k: &Kind) -> (Vec<String>, usize) {
        let (offer, at) = self.offer(k);
        let names = offer.iter().map(|&i| self.apps[i].name.clone());
        match at {
            Some(at) => (names.collect(), at),
            None => (
                std::iter::once(NONE_ROW.to_string()).chain(names).collect(),
                0,
            ),
        }
    }

    /// Row `row` of kind `k`'s combo picked: its program becomes the
    /// default of the kind's types. False for "(None)" or no change.
    pub fn pick(&mut self, k: &Kind, row: usize) -> bool {
        let (offer, at) = self.offer(k);
        let Some(&app) = (match at {
            Some(_) => offer.get(row),
            None => row.checked_sub(1).and_then(|r| offer.get(r)),
        }) else {
            return false;
        };
        if at.map(|a| offer[a]) == Some(app) {
            return false;
        }
        let entry = &self.apps[app];
        let id = entry.id.clone();
        let declares = |m: &str| entry.mime_types.iter().any(|t| t == m);
        let types: Vec<&str> = (k.types.iter().enumerate())
            .filter(|(i, m)| *i < k.always || declares(m))
            .map(|(_, m)| *m)
            .collect();
        for m in types {
            self.set(m, Some(&id));
        }
        true
    }

    /// The Terminal combo's rows and the picked one for the `terminal`
    /// key's `current` value. A command no installed terminal runs is a
    /// row of its own after them, so it is not silently dropped.
    pub fn terminal_rows(&self, current: &str) -> (Vec<String>, usize) {
        let mut rows: Vec<String> = self.terminals.iter().map(|t| t.name.clone()).collect();
        match self.terminal_at(current) {
            Some(i) => (rows, i),
            None => {
                rows.push(current.to_string());
                let n = rows.len() - 1;
                (rows, n)
            }
        }
    }

    /// The terminal the `terminal` key names: the same program, by path
    /// or by name.
    fn terminal_at(&self, current: &str) -> Option<usize> {
        let base = |c: &str| {
            let first = c.split_whitespace().next().unwrap_or("");
            first.rsplit('/').next().unwrap_or(first).to_string()
        };
        (self.terminals.iter())
            .position(|t| t.command == current || base(&t.command) == base(current))
    }

    /// The `terminal` value row `row` of the Terminal combo stands for;
    /// None for the row that is the current value itself.
    pub fn terminal_pick(&self, row: usize) -> Option<&str> {
        self.terminals.get(row).map(|t| t.command.as_str())
    }

    /// Show the types whose extension, type, description or program
    /// holds `filter`, case folded.
    pub fn set_filter(&mut self, filter: &str) {
        self.filter = filter.to_lowercase();
        self.refilter();
    }

    fn refilter(&mut self) {
        let f = self.filter.clone();
        self.shown = (0..self.types.len())
            .filter(|&i| {
                let t = &self.types[i];
                f.is_empty()
                    || t.ext.as_deref().is_some_and(|e| e.contains(&f))
                    || t.mime.contains(&f)
                    || t.description.to_lowercase().contains(&f)
                    || self.program_name(&t.mime).to_lowercase().contains(&f)
            })
            .collect();
    }

    pub fn program_name(&self, mime: &str) -> String {
        self.current(mime)
            .map(|i| self.apps[i].name.clone())
            .unwrap_or_default()
    }

    pub fn type_at(&self, row: usize) -> Option<&FileType> {
        self.shown.get(row).and_then(|&i| self.types.get(i))
    }
}

fn terminals(apps: &[AppEntry]) -> Vec<Terminal> {
    let mut out: Vec<Terminal> = apps
        .iter()
        .filter(|a| a.shown && a.in_category("TerminalEmulator"))
        .filter_map(|a| {
            let command = reprisefs::apps::exec_argv(&a.exec_line)
                .into_iter()
                .next()?;
            Some(Terminal {
                name: a.name.clone(),
                command,
            })
        })
        .collect();
    out.sort_by_key(|t| t.name.to_lowercase());
    let mut seen = Vec::new();
    out.retain(|t| {
        let fresh = !seen.contains(&t.command);
        seen.push(t.command.clone());
        fresh
    });
    out
}

/// The types the list shows: every type a shown program declares or a
/// list names a default for, but the URL schemes (the combos have those)
/// and the media kinds `x-content/*`, by extension, then the types with
/// none by name.
fn file_types(
    apps: &[AppEntry],
    lists: &[String],
    ext: &std::collections::HashMap<String, String>,
    describe: &dyn Fn(&str) -> Option<String>,
) -> Vec<FileType> {
    let declared = apps
        .iter()
        .filter(|a| a.shown)
        .flat_map(|a| a.mime_types.iter().map(|m| m.to_ascii_lowercase()));
    let listed = lists.iter().flat_map(|t| defaults_in(t));
    let mut mimes: Vec<String> = declared
        .chain(listed)
        .filter(|m| {
            m.contains('/') && !m.starts_with("x-scheme-handler/") && !m.starts_with("x-content/")
        })
        .collect();
    mimes.sort();
    mimes.dedup();
    let mut out: Vec<FileType> = mimes
        .into_iter()
        .map(|mime| FileType {
            ext: ext.get(&mime).cloned(),
            description: describe(&mime).unwrap_or_else(|| mime.clone()),
            mime,
        })
        .collect();
    out.sort_by(|a, b| {
        (a.ext.is_none(), a.ext.as_ref().unwrap_or(&a.mime))
            .cmp(&(b.ext.is_none(), b.ext.as_ref().unwrap_or(&b.mime)))
    });
    out
}

fn defaults_in(text: &str) -> Vec<String> {
    let mut in_defaults = false;
    let mut out = Vec::new();
    for l in text.lines() {
        let t = l.trim();
        if t.starts_with('[') {
            in_defaults = t == "[Default Applications]";
        } else if in_defaults && let Some((k, _)) = t.split_once('=') {
            out.push(k.trim().to_ascii_lowercase());
        }
    }
    out
}

/// Put `d`'s programs in the page's combos, the pending terminal
/// `terminal`'s program picked and its arguments in their field, and grey
/// Change... and Restore Default as the picked row `row` allows.
pub fn fill(form: &mut Form, d: &Defaults, terminal: &str, row: Option<usize>) {
    for k in &KINDS {
        let (rows, at) = d.kind_rows(k);
        set_combo(form, k.id, &rows, at);
    }
    let (program, args) = split_command(terminal);
    let (rows, at) = d.terminal_rows(&program);
    set_combo(form, TERMINAL, &rows, at);
    if let Some(Control::Edit { edit, first, .. }) =
        form.item_mut(TERMINAL_ARGS).map(|i| &mut i.control)
        && edit.text.trim() != args
    {
        *edit = reprisekit::textedit::TextEdit::new(args, false);
        first.set(0);
    }
    sync_buttons(form, d, row);
}

/// The `terminal` key split into its program, unquoted, and the arguments after it.
pub fn split_command(command: &str) -> (String, &str) {
    reprisefs::apps::exec_program_and_rest(command)
}

/// The `terminal` key for `program` run with `args`.
pub fn join_command(program: &str, args: &str) -> String {
    format!("{} {}", reprisefs::apps::exec_quote(program), args.trim())
        .trim()
        .to_string()
}

/// Change... wants a picked type; Restore Default one the user set. A
/// greyed button is no tab stop, so its focus goes to Change... while
/// that is live, else to nothing.
pub fn sync_buttons(form: &mut Form, d: &Defaults, row: Option<usize>) {
    let t = row.and_then(|r| d.type_at(r));
    let change = t.is_some();
    form.set_enabled(CHANGE, change);
    form.set_enabled(RESTORE, t.is_some_and(|t| d.user_set(&t.mime)));
    let focused = form.focus().and_then(|f| form.items.get(f)).map(|i| i.id);
    let greyed = |id: &str| {
        matches!(
            form.item(id).map(|i| &i.control),
            Some(Control::Button { enabled: false, .. })
        )
    };
    if focused.is_some_and(|id| (id == CHANGE || id == RESTORE) && greyed(id)) {
        if change {
            form.focus_id(CHANGE);
        } else {
            form.set_focus(None);
        }
    }
}

fn set_combo(form: &mut Form, id: &str, rows: &[String], at: usize) {
    if let Some(Control::Combo(c)) = form.item_mut(id).map(|i| &mut i.control) {
        set_combo_rows(c, rows, at);
    }
}

fn set_combo_rows(c: &mut Combo, rows: &[String], at: usize) {
    let refs: Vec<&str> = rows.iter().map(String::as_str).collect();
    let enabled = c.enabled;
    *c = Combo::new(&refs, at, rows.len().clamp(1, DROP_ROWS));
    c.enabled = enabled;
}

/// Refill `lv` from the types `d` shows, the type `keep` picked again
/// where it is still shown.
pub fn fill_list(lv: &mut ListView, d: &Defaults, keep: Option<&str>) {
    lv.rows = d
        .shown
        .iter()
        .map(|&i| {
            let t = &d.types[i];
            Row {
                icon: None,
                cells: vec![
                    t.ext.clone().unwrap_or_else(|| t.mime.clone()),
                    t.description.clone(),
                    d.program_name(&t.mime),
                ],
            }
        })
        .collect();
    let at = keep.and_then(|m| (0..d.shown.len()).find(|&r| d.types[d.shown[r]].mime == m));
    lv.selection.set_only(at);
    lv.top = lv.top.min(lv.rows.len().saturating_sub(1));
}

pub fn picked<'a>(lv: &ListView, d: &'a Defaults) -> Option<&'a FileType> {
    lv.selection.focus.and_then(|r| d.type_at(r))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::collections::HashMap;

    fn app(id: &str, name: &str, exec: &str, mimes: &[&str], categories: &[&str]) -> AppEntry {
        AppEntry {
            name: name.into(),
            exec_line: exec.into(),
            terminal: false,
            id: id.into(),
            icon: None,
            startup_wm_class: None,
            categories: categories.iter().map(|c| c.to_string()).collect(),
            mime_types: mimes.iter().map(|m| m.to_string()).collect(),
            shown: true,
        }
    }

    const WEB: [&str; 4] = [
        "text/html",
        "x-scheme-handler/http",
        "x-scheme-handler/https",
        "application/xhtml+xml",
    ];

    pub(crate) fn defaults() -> Defaults {
        let apps = vec![
            app("firefox.desktop", "Firefox", "firefox %u", &WEB, &[]),
            app(
                "chromium.desktop",
                "Chromium",
                "chromium %U",
                &WEB[..3],
                &[],
            ),
            app(
                "notepad.desktop",
                "Notepad",
                "notepad %f",
                &["text/plain"],
                &[],
            ),
            app(
                "zed.desktop",
                "Zed",
                "zed %F",
                &["text/plain", "text/x-rust"],
                &[],
            ),
            app(
                "foot.desktop",
                "Foot",
                "foot",
                &[],
                &["System", "TerminalEmulator"],
            ),
            app(
                "kitty.desktop",
                "kitty",
                "/usr/bin/kitty",
                &[],
                &["TerminalEmulator"],
            ),
        ];
        let user = "[Default Applications]\n\
                    x-scheme-handler/https=chromium.desktop\n\
                    x-scheme-handler/http=chromium.desktop\n";
        let desktop =
            "[Default Applications]\ntext/plain=notepad.desktop\nimage/png=gone.desktop\n";
        let ext: HashMap<String, String> = [("text/plain", ".txt"), ("text/x-rust", ".rs")]
            .iter()
            .map(|(m, e)| (m.to_string(), e.to_string()))
            .collect();
        Defaults::new(
            apps,
            MimeTree::default(),
            vec![String::new(), user.into(), desktop.into()],
            1,
            &ext,
            &|m| (m == "text/plain").then(|| "Plain text document".to_string()),
        )
    }

    fn kind(id: &str) -> &'static Kind {
        KINDS.iter().find(|k| k.id == id).unwrap()
    }

    fn id_of(d: &Defaults, mime: &str) -> Option<String> {
        d.current(mime).map(|i| d.apps[i].id.clone())
    }

    #[test]
    fn a_browsed_program_gets_an_entry_and_becomes_the_default() {
        let dir = std::env::temp_dir().join(format!("reprise-dp-browse-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut d = defaults();
        let n = d.apps.len();
        assert_eq!(
            d.chosen("notepad.desktop", "text/plain", None).unwrap(),
            "notepad.desktop",
            "an installed program is taken as it is"
        );
        let id = d.chosen("/opt/view", "text/plain", Some(&dir)).unwrap();
        assert_eq!(id, "reprise-userapp-view.desktop");
        assert!(dir.join(&id).is_file(), "the entry is written");
        d.set("text/plain", Some(&id));
        assert_eq!(
            d.current("text/plain").map(|i| d.apps[i].id.as_str()),
            Some(id.as_str())
        );
        assert_eq!(
            d.chosen("/opt/view", "text/x-rust", Some(&dir)).unwrap(),
            id
        );
        assert_eq!(d.apps.len(), n + 1, "picked again: the entry is reused");
        assert_eq!(d.apps[n].mime_types, ["text/plain", "text/x-rust"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_browser_pick_sets_its_schemes_and_web_pages() {
        let mut d = defaults();
        let rows = d.kind_rows(kind("dp_browser"));
        assert_eq!(rows, (vec!["Chromium".into(), "Firefox".into()], 0));
        assert!(!d.dirty());
        assert!(d.pick(kind("dp_browser"), 1), "Firefox");
        for m in WEB {
            assert_eq!(id_of(&d, m).as_deref(), Some("firefox.desktop"), "{m}");
        }
        assert!(d.dirty());
        assert!(!d.pick(kind("dp_browser"), 1), "picked again: no change");
        d.revert();
        assert_eq!(
            id_of(&d, "x-scheme-handler/https").as_deref(),
            Some("chromium.desktop")
        );
        assert!(!d.dirty());
    }

    #[test]
    fn a_pick_sets_the_optional_types_its_program_declares() {
        let mut d = defaults();
        d.pick(kind("dp_browser"), 1);
        d.pick(kind("dp_browser"), 0);
        let set: Vec<&str> = d.edits.iter().map(|(m, _)| m.as_str()).collect();
        assert_eq!(
            set,
            [
                "application/xhtml+xml",
                "x-scheme-handler/https",
                "x-scheme-handler/http",
                "text/html"
            ],
            "one edit per type, the latest last"
        );
        assert_eq!(
            id_of(&d, "application/xhtml+xml").as_deref(),
            Some("firefox.desktop")
        );
        assert_eq!(id_of(&d, "text/html").as_deref(), Some("chromium.desktop"));
    }

    #[test]
    fn a_kind_with_no_program_offers_none_first() {
        let mut d = defaults();
        let rows = d.kind_rows(kind("dp_mail"));
        assert_eq!(rows, (vec![NONE_ROW.to_string()], 0));
        assert!(!d.pick(kind("dp_mail"), 0), "(None) sets nothing");
        let rows = d.kind_rows(kind("dp_editor"));
        assert_eq!(rows, (vec!["Notepad".into(), "Zed".into()], 0));
    }

    #[test]
    fn restore_default_drops_the_users_choice() {
        let mut d = defaults();
        assert!(d.user_set("x-scheme-handler/https"));
        assert!(!d.user_set("text/plain"), "the desktop's list set it");
        d.set("text/plain", Some("zed.desktop"));
        assert!(d.user_set("text/plain"));
        d.set("text/plain", None);
        assert!(!d.user_set("text/plain"));
        assert_eq!(id_of(&d, "text/plain").as_deref(), Some("notepad.desktop"));
        assert!(!d.dirty(), "set and taken back is no change");
        d.set("x-scheme-handler/https", None);
        assert_eq!(
            id_of(&d, "x-scheme-handler/https").as_deref(),
            Some("chromium.desktop"),
            "with no default, the first program by name that declares it"
        );
        assert!(d.dirty());
        d.applied("[Default Applications]\nx-scheme-handler/http=chromium.desktop\n".into());
        assert!(!d.dirty() && d.edits.is_empty());
        assert!(!d.user_set("x-scheme-handler/https"));
    }

    #[test]
    fn the_terminal_combo_finds_the_key_by_program() {
        let d = defaults();
        let names = vec!["Foot".to_string(), "kitty".to_string()];
        assert_eq!(d.terminal_rows("foot"), (names.clone(), 0));
        assert_eq!(d.terminal_rows("kitty"), (names.clone(), 1), "by name");
        assert_eq!(d.terminal_rows("/usr/bin/foot"), (names, 0), "by path");
        let (rows, at) = d.terminal_rows("wezterm start");
        assert_eq!((rows[2].as_str(), at), ("wezterm start", 2));
        assert_eq!(d.terminal_pick(1), Some("/usr/bin/kitty"));
        assert_eq!(d.terminal_pick(2), None, "the key's own value");
    }

    #[test]
    fn a_terminal_command_splits_into_its_program_and_arguments_and_joins_back() {
        assert_eq!(split_command("foot"), ("foot".to_string(), ""));
        assert_eq!(
            split_command(" foot  -e tmux new "),
            ("foot".to_string(), "-e tmux new")
        );
        assert_eq!(join_command("foot", "-e tmux"), "foot -e tmux");
        assert_eq!(join_command("/usr/bin/kitty", "  "), "/usr/bin/kitty");
        assert_eq!(
            split_command("\"/opt/My Term/term\" --hold"),
            ("/opt/My Term/term".to_string(), "--hold"),
            "a quoted program is one program"
        );
        let key = join_command("/home/u/My Apps/x.AppImage", "--hold");
        assert_eq!(key, "\"/home/u/My Apps/x.AppImage\" --hold");
        assert_eq!(
            split_command(&key),
            ("/home/u/My Apps/x.AppImage".to_string(), "--hold")
        );
    }

    #[test]
    fn the_terminal_row_picks_the_program_and_the_field_holds_its_arguments() {
        let d = defaults();
        let mut form = page(Area::MIN);
        let args = |f: &Form| match f.item(TERMINAL_ARGS).map(|i| &i.control) {
            Some(Control::Edit { edit, .. }) => edit.text.clone(),
            _ => panic!("the arguments field is an edit"),
        };
        fill(
            &mut form,
            &d,
            "kitty --single-instance -o font_size=9",
            None,
        );
        assert_eq!(form.combo_selected(TERMINAL), Some(1), "kitty");
        assert_eq!(args(&form), "--single-instance -o font_size=9");
        let (rows, at) = d.terminal_rows("wezterm");
        fill(&mut form, &d, "wezterm start --cwd .", None);
        assert_eq!(form.combo_selected(TERMINAL), Some(at));
        assert_eq!(
            rows[at], "wezterm",
            "an unknown program's own row, without its arguments"
        );
        assert_eq!(args(&form), "start --cwd .");
        fill(&mut form, &d, "foot", None);
        assert_eq!(args(&form), "", "no arguments");
    }

    #[test]
    fn the_file_types_are_declared_or_listed_and_filter_by_any_column() {
        let mut d = defaults();
        let types: Vec<(&str, Option<&str>)> = (d.types.iter())
            .map(|t| (t.mime.as_str(), t.ext.as_deref()))
            .collect();
        assert_eq!(
            types,
            [
                ("text/x-rust", Some(".rs")),
                ("text/plain", Some(".txt")),
                ("application/xhtml+xml", None),
                ("image/png", None),
                ("text/html", None),
            ],
            "by extension, then by type; no URL schemes"
        );
        assert_eq!(d.types[1].description, "Plain text document");
        assert_eq!(
            d.types[3].description, "image/png",
            "no description: the type"
        );
        assert_eq!(d.program_name("image/png"), "", "its program is gone");
        d.set_filter("PLAIN");
        assert_eq!(d.shown, [1]);
        d.set_filter("zed");
        assert_eq!(d.shown, [0], "by program");
        d.set_filter(".rs");
        assert_eq!(d.type_at(0).map(|t| t.mime.as_str()), Some("text/x-rust"));
        d.set_filter("");
        assert_eq!(d.shown.len(), 5);
    }

    fn drawn(dropped: bool) -> (reprisekit::raster::Canvas, Rect, Rect) {
        use crate::registry::{self, Page, Sources};
        use reprisekit::controls::propsheet;
        use reprisekit::text::TextRenderer;
        use reprisekit::theme::{FontConfig, Theme};
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let px = theme.fonts.size_px;
        let mut sheet = registry::sheet(
            &crate::model::Settings::default(),
            &Sources {
                icon_themes: &[],
                gtk_themes: &[],
                qt_tools: false,
                cursor_themes: &[],
                sound_themes: &[],
                layouts: &[],
                heads: &[],
            },
        );
        sheet.tabs.open = Page::DefaultPrograms.index();
        let d = defaults();
        let mut lv = new_list();
        fill_list(&mut lv, &d, Some("text/plain"));
        let page = &mut sheet.pages[Page::DefaultPrograms.index()];
        fill(page, &d, "foot", lv.selection.focus);
        if dropped && let Some(Control::Combo(c)) = page.item_mut(TERMINAL).map(|i| &mut i.control)
        {
            let rows: Vec<String> = (0..DROP_ROWS).map(|i| format!("Terminal {i}")).collect();
            set_combo_rows(c, &rows, 0);
            c.toggle();
        }
        let mut c = reprisekit::raster::Canvas::new(sheet.size.0 as u32, sheet.size.1 as u32);
        c.fill_rect(c.bounds(), theme.colors.button_face);
        propsheet::draw(&mut c, &text, &theme.paint(), &sheet, true, px);
        let list = sheet.on_page(list(Area::MIN));
        crate::pages::draw_list(&mut c, &text, &theme, &lv, list, px, 1);
        propsheet::draw_popups(&mut c, &text, &theme.paint(), &sheet, px);
        let terminal = page_rect(&sheet, TERMINAL);
        (c, list, terminal)
    }

    fn page_rect(sheet: &reprisekit::controls::propsheet::PropSheet, id: &str) -> Rect {
        sheet.pages[crate::registry::Page::DefaultPrograms.index()]
            .item(id)
            .unwrap()
            .rect
    }

    #[test]
    fn the_page_geometry() {
        let f = page(Area::MIN);
        let at = |id| f.item(id).unwrap().rect;
        assert_eq!(at("dp_browser_label"), Rect::new(20, 48, 96, 13));
        assert_eq!(at("dp_browser"), Rect::new(120, 44, 258, 21));
        assert_eq!(at(TERMINAL_LABEL), Rect::new(20, 209, 96, 13));
        assert_eq!(at(TERMINAL), Rect::new(120, 205, 258, 21));
        assert_eq!(at(TERMINAL_ARGS_LABEL), Rect::new(20, 232, 140, 13));
        assert_eq!(at(TERMINAL_ARGS), Rect::new(164, 228, 214, 20));
        assert_eq!(at(DRIVE_INSERT_LABEL), Rect::new(20, 255, 140, 13));
        assert_eq!(at(DRIVE_INSERT), Rect::new(164, 251, 214, 21));
        assert_eq!(at(SCRIPT_OPEN_LABEL), Rect::new(20, 278, 140, 13));
        assert_eq!(at(SCRIPT_OPEN), Rect::new(164, 274, 214, 21));
        assert_eq!(at(SCRIPT_END_LABEL), Rect::new(20, 301, 140, 13));
        assert_eq!(at(SCRIPT_END), Rect::new(164, 297, 214, 21));
        assert_eq!(at(CONFIRM_SCRIPTS), Rect::new(20, 324, 13, 13));
        assert_eq!(at("dp_types_label"), Rect::new(20, 359, 120, 13));
        assert_eq!(at(FILTER), Rect::new(228, 355, 150, 20));
        assert_eq!(at(CHANGE), Rect::new(201, 446, 75, 23));
        assert_eq!(at(RESTORE), Rect::new(282, 446, 96, 23));
        assert_eq!(
            at(RESTORE).right(),
            list(Area::MIN).right(),
            "flush with the list"
        );
        assert!(at(RESTORE).bottom() <= 492, "inside the page");
    }

    #[test]
    fn the_script_row_pins_its_pixels_in_every_look() {
        if !reprisekit::text::test_tahoma() {
            return;
        }
        use reprisekit::controls::form;
        use reprisekit::interface::Interface;
        use reprisekit::raster::Canvas;
        fn fnv(c: &Canvas) -> u64 {
            c.pixels.iter().fold(0xcbf29ce484222325, |h, p| {
                (h ^ *p as u64).wrapping_mul(0x100000001b3)
            })
        }
        let row = Rect::new(X, 297, 358, 21);
        let mut got = Vec::new();
        for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
            let theme = reprisekit::theme::Theme {
                access_keys: true,
                ..reprisekit::interfaces::theme_for(i)
            };
            let text = reprisekit::text::TextRenderer::load(&theme.fonts);
            let mut hashes = [0; 2];
            for (k, hash) in hashes.iter_mut().enumerate() {
                let mut f = page(Area::MIN);
                f.set_combo(SCRIPT_END, k);
                let mut c = Canvas::new(row.right() as u32, row.bottom() as u32);
                c.fill_rect(c.bounds(), theme.colors.button_face);
                form::draw(
                    &mut c,
                    &text,
                    &theme.paint(),
                    &f,
                    false,
                    theme.fonts.size_px,
                );
                *hash = fnv(&c.copy_rect(row));
            }
            assert_ne!(hashes[0], hashes[1], "{i:?}: each choice shows its text");
            got.push(hashes);
        }
        assert_eq!(
            got,
            [
                [0xc3fdac5f80ffde45, 0xb27f9e19039ad678],
                [0xe3854151f8c565c6, 0x3806333260ae309f],
                [0xc874dd913e0019a3, 0x538adfb3d94f93ca],
            ],
            "Classic, Blue, Glass: keep open, close"
        );
    }

    #[test]
    fn the_page_draws_the_list_under_a_dropped_combo() {
        let cs = reprisekit::theme::Theme::default().colors;
        let (c, list, terminal) = drawn(false);
        assert_eq!((list.x, list.y), (166, 381));
        let header = (list.x + 80, list.y + 6);
        let row = |r: i32| list.y + 2 + 17 + 17 * r + 8;
        let lit = |c: &reprisekit::raster::Canvas, y| {
            (list.x..list.x + 60).any(|x| c.get(x, y) == cs.hilight.argb())
        };
        assert_eq!(
            c.get(header.0, header.1),
            cs.button_face.argb(),
            "the header"
        );
        assert!(lit(&c, row(1)), "the picked row");
        assert!(!lit(&c, row(0)), "the row above it");
        assert_eq!(c.get(list.x + 3, row(0)), cs.window.argb(), "the body");
        let field = (terminal.right() - 30, terminal.y + 10);
        assert_eq!(c.get(field.0, field.1), cs.window.argb(), "the field");
        let (open, _, _) = drawn(true);
        assert!(
            (list.y..list.bottom()).any(|y| open.get(list.x + 330, y) != c.get(list.x + 330, y)),
            "the dropped list covers the list's top"
        );
    }

    #[test]
    fn the_list_keeps_its_pick_and_the_buttons_follow_it() {
        let mut d = defaults();
        let mut form = page(Area::MIN);
        let mut lv = new_list();
        fill_list(&mut lv, &d, None);
        assert_eq!(lv.rows[1].cells, [".txt", "Plain text document", "Notepad"]);
        fill(&mut form, &d, "foot", None);
        assert_eq!(form.combo_selected(TERMINAL), Some(0));
        let lit = |f: &Form, id| match f.item(id).map(|i| &i.control) {
            Some(Control::Button { enabled, .. }) => *enabled,
            _ => panic!("{id} is a button"),
        };
        assert!(
            !lit(&form, CHANGE) && !lit(&form, RESTORE),
            "nothing picked"
        );
        fill_list(&mut lv, &d, Some("text/plain"));
        assert_eq!(lv.selection.focus, Some(1));
        sync_buttons(&mut form, &d, lv.selection.focus);
        assert!(lit(&form, CHANGE) && !lit(&form, RESTORE));
        d.set("text/plain", Some("zed.desktop"));
        d.set_filter("text");
        fill_list(&mut lv, &d, Some("text/plain"));
        assert_eq!(lv.rows[1].cells[2], "Zed");
        sync_buttons(&mut form, &d, lv.selection.focus);
        assert!(lit(&form, RESTORE), "the user chose its program");
        form.focus_id(RESTORE);
        d.set("text/plain", None);
        sync_buttons(&mut form, &d, lv.selection.focus);
        assert!(!lit(&form, RESTORE));
        let focused = form.focus().map(|f| form.items[f].id);
        assert_eq!(focused, Some(CHANGE), "the focus leaves the greyed button");
    }
}
