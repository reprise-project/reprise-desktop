//! Properties sheets: what OK and Apply write, and the attribute confirmation they raise.

use super::*;

impl Dialog {
    fn props_mut(&mut self) -> Option<(&mut SheetBase, &mut reprisekit::controls::form::Form)> {
        let Style::Form(f) = &mut self.style else {
            return None;
        };
        let FormKind::Properties(base) = &mut f.kind else {
            return None;
        };
        Some((base, f.body.pages_mut().get_mut(0)?))
    }

    /// After an OK or Apply wrote `w`: the sheet's base takes each field
    /// as it was written, so Apply greys for those and stays lit for an
    /// edit made while the write ran or a field whose write failed.
    pub fn sheet_applied(&mut self, w: &Written) {
        let state = |on: bool| if on { CheckState::On } else { CheckState::Off };
        let Some((base, _)) = self.props_mut() else {
            return;
        };
        base.path = w.path.clone();
        if let Some(on) = w.attrs.read_only {
            base.read_only = state(on);
        }
        if let Some(on) = w.attrs.executable {
            base.executable = on;
        }
        // A cleared Hidden took the name off the folder's .hidden.
        if let Some(h) = w.hidden {
            base.hidden = h;
            base.listed &= h;
        }
        if let Some(robots) = base.robots.as_mut() {
            for (k, moved) in [w.attrs.backup, w.attrs.index].into_iter().enumerate() {
                if let Some(on) = moved {
                    robots[k] = state(on);
                }
            }
        }
        if let Some(icon) = &w.folder_icon {
            base.own_icon = icon.clone();
        }
        if let Some(fields) = base.summary.as_mut() {
            for (k, v) in &w.summary {
                if let Some(f) = fields.get_mut(*k) {
                    *f = v.clone();
                }
            }
        }
        for (key, v) in &w.launcher {
            if let Some(l) = base.launcher.as_mut() {
                l.set(*key, v);
            }
            if *key == reprisefs::launcher::Key::Icon {
                base.own_icon = Some(v.clone());
            }
        }
        if let Some(f) = self.form_mut() {
            sync_apply(f);
        }
    }

    /// After an OK or Apply that rewrote a shortcut's keys: rebuild the sheet over `info` with
    /// `icon` in the header, keeping caption, place and tab. The host fits it to the theme again.
    pub fn sheet_reread(&mut self, info: &reprisefs::props::FileInfo, icon: &str) {
        let Some(open) = self
            .form()
            .and_then(|f| f.sheet())
            .and_then(|s| s.tabs.labels.get(s.tabs.open).cloned())
        else {
            return;
        };
        let fresh = Dialog::properties(info, icon, (0, 0));
        self.style = fresh.style;
        self.icon = fresh.icon;
        if let Some(sheet) = self.form_mut().and_then(|f| f.sheet_mut())
            && let Some(i) = sheet.tabs.labels.iter().position(|l| *l == open)
        {
            sheet.open_tab(i);
        }
    }

    /// After Apply wrote a set's changes: its items are at `paths` now
    /// and the boxes that settled are where they start, so Apply greys
    /// again.
    pub fn set_applied(&mut self, paths: Vec<std::path::PathBuf>) {
        use reprisekit::controls::filesheet;
        let Some(f) = self.form_mut() else {
            return;
        };
        let (FormKind::SetProperties(set), Some(form)) = (&mut f.kind, f.body.pages().first())
        else {
            return;
        };
        set.paths = paths;
        let settled = |s: CheckState| (s != CheckState::Mixed).then_some(s.is_on());
        if let Some(on) = settled(filesheet::read_only(form)) {
            set.read_only = Some(on);
        }
        if let Some(on) = settled(filesheet::hidden_state(form)) {
            set.hidden = Some(on);
        }
        set.icon_chosen = None;
        if let (Some(adv), Some(read)) = (set.adv.take(), set.set.robots.as_mut()) {
            for (k, flag) in read.iter_mut().enumerate() {
                if let Some(on) = settled(adv[k]) {
                    *flag = Some(on);
                }
            }
        }
        sync_apply(f);
    }

    /// Change Icon's OK over this sheet: the icon waits here until the
    /// sheet's OK or Apply writes it, and Customize shows it meanwhile.
    /// False when this is no Properties sheet.
    pub fn icon_chosen(&mut self, icon: Option<String>) -> bool {
        let launcher = matches!(
            self.form().map(|f| &f.kind),
            Some(FormKind::Properties(b)) if b.launcher.is_some()
        );
        // A launcher's Shortcut page wears the pick in its header.
        if launcher && let Some(name) = &icon {
            self.icon = Some(name.clone());
        }
        let Some(f) = self.form_mut() else {
            return false;
        };
        match &mut f.kind {
            FormKind::Properties(base) => base.icon_chosen = Some(icon),
            FormKind::SetProperties(set) => set.icon_chosen = Some(icon),
            _ => return false,
        }
        sync_apply(f);
        true
    }

    /// The Change Icon box for folder `name`, or a launcher's with `launcher`; Restore Defaults on
    /// a folder's. OK hands the pick to the sheet.
    pub fn change_icon(
        name: &str,
        source: &str,
        icons: reprisekit::controls::iconlist::IconList,
        launcher: bool,
    ) -> Dialog {
        use reprisekit::controls::filesheet;
        let form = filesheet::change_icon_form(source, icons, !launcher);
        let title = if launcher {
            "Change Icon".to_string()
        } else {
            format!("Change Icon for {name} Folder")
        };
        Dialog {
            help: true,
            ..Dialog::new(
                title,
                Style::Form(Box::new(FormDialog::from_client(
                    filesheet::CHANGE_ICON_SIZE,
                    form,
                    FormKind::ChangeIcon,
                ))),
                Placement::Center,
            )
        }
    }

    /// Advanced Attributes' OK over this sheet: the archive and index
    /// boxes wait here until the sheet's OK or Apply writes them, and
    /// Apply lights while they differ from the file. False when this is
    /// no Properties sheet.
    pub fn advanced_chosen(&mut self, chosen: [CheckState; 2]) -> bool {
        let Some(f) = self.form_mut() else {
            return false;
        };
        match &mut f.kind {
            FormKind::Properties(base) => base.adv = Some(chosen),
            FormKind::SetProperties(set) => set.adv = Some(chosen),
            _ => return false,
        }
        sync_apply(f);
        true
    }

    pub fn set_paths(&self) -> Option<&[std::path::PathBuf]> {
        match &self.form()?.kind {
            FormKind::SetProperties(s) => Some(&s.paths),
            _ => None,
        }
    }
}

/// What a folder's or a set's Properties sheet asked to write, held back
/// until the Confirm Attribute Changes dialog says how far it reaches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttrConfirm {
    pub write: AttrWrite,
    /// The attribute changes asked for besides Hidden (Read-only, and
    /// archive and index from Advanced Attributes).
    pub attrs: reprisefs::props::AttrMoves,
    /// Whether the sheet closes after the write (its OK rather than Apply).
    pub close: bool,
}

/// The rest of the write Confirm Attribute Changes holds: a folder's
/// rename and the Hidden state it takes, or a set's items and theirs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AttrWrite {
    One {
        path: std::path::PathBuf,
        name: Option<String>,
        hidden: Option<bool>,
        folder_icon: Option<Option<String>>,
    },
    Set {
        paths: Vec<std::path::PathBuf>,
        hidden: Option<bool>,
        folder_icon: Option<Option<String>>,
    },
}

impl AttrConfirm {
    /// The sheet's write with `tree`, the answer, as how far Read-only
    /// and Hidden reach.
    pub(super) fn answered(&self, tree: bool) -> Outcome {
        let (attrs, close, recurse) = (self.attrs, self.close, Some(tree));
        match &self.write {
            AttrWrite::One {
                path,
                name,
                hidden,
                folder_icon,
            } => Outcome::Properties {
                path: path.clone(),
                name: name.clone(),
                attrs,
                hidden: *hidden,
                // Only a folder is asked, and a folder has no Summary.
                summary: Vec::new(),
                launcher: Vec::new(),
                folder_icon: folder_icon.clone(),
                close,
                recurse,
            },
            AttrWrite::Set {
                paths,
                hidden,
                folder_icon,
            } => Outcome::SetProperties {
                paths: paths.clone(),
                attrs,
                hidden: *hidden,
                folder_icon: folder_icon.clone(),
                close,
                recurse,
            },
        }
    }

    /// The Hidden state asked for, None when the box did not move.
    pub fn hidden(&self) -> Option<bool> {
        match &self.write {
            AttrWrite::One { hidden, .. } | AttrWrite::Set { hidden, .. } => *hidden,
        }
    }

    pub(super) fn path(&self) -> Option<&std::path::Path> {
        match &self.write {
            AttrWrite::One { path, .. } => Some(path),
            AttrWrite::Set { paths, .. } => paths.first().map(|p| p.as_path()),
        }
    }
}

/// Properties sheet: OK/Enter/Apply ask the host to write; Change.../Advanced... open dialogs;
/// Apply lights exactly while there is something to write.
pub(super) fn props_event(
    fd: &mut FormDialog,
    _page: usize,
    ev: reprisekit::controls::form::Event,
) -> Outcome {
    use reprisekit::controls::filesheet::{ADVANCED, CHANGE};
    use reprisekit::controls::form::Event;
    use reprisekit::controls::propsheet::{APPLY, CANCEL, OK};
    let FormKind::Properties(base) = &fd.kind else {
        return Outcome::Nothing;
    };
    // The changes are the General page's, whichever page is open.
    let Some(form) = fd.pages().first() else {
        return Outcome::Nothing;
    };
    let summary = summary_changes(fd.pages(), base);
    let launcher = launcher_changes(fd.pages(), base);
    // A launcher's icon is one of its keys.
    let folder_icon = icon_move(base).filter(|_| base.launcher.is_none());
    let rest = summary.is_empty() && folder_icon.is_none() && launcher.is_empty();
    let write = |close: bool| match props_changes(form, base) {
        (None, attrs, None) if attrs.is_empty() && rest && close => Outcome::Close,
        (None, attrs, None) if attrs.is_empty() && rest => Outcome::Nothing,
        (name, attrs, hidden) => Outcome::Properties {
            path: base.path.clone(),
            name,
            attrs,
            hidden,
            summary: summary.clone(),
            folder_icon: folder_icon.clone(),
            launcher: launcher.clone(),
            close,
            recurse: None,
        },
    };
    match ev {
        Event::Default | Event::Pressed(OK) => write(true),
        Event::Pressed(APPLY) => write(false),
        Event::Cancel | Event::Pressed(CANCEL) => Outcome::Close,
        Event::Pressed(CHANGE) => Outcome::ChangeDefault(base.path.clone()),
        Event::Pressed(reprisekit::controls::filesheet::CHANGE_ICON) => Outcome::ChangeIcon {
            name: base
                .path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            current: base
                .icon_chosen
                .clone()
                .unwrap_or_else(|| base.own_icon.clone()),
            launcher: base.launcher.is_some(),
        },
        Event::Pressed(reprisekit::controls::filesheet::FIND_TARGET) => {
            find_target(fd.pages(), base)
        }
        Event::Pressed(ADVANCED) => Outcome::AdvancedAttributes {
            subject: reprisekit::controls::filesheet::Subject::single(base.is_dir),
            robots: base.robots.map(|opened| base.adv.unwrap_or(opened)),
        },
        Event::Changed(_) | Event::Redraw => {
            sync_apply(fd);
            Outcome::Redraw
        }
        Event::None | Event::Pressed(_) => Outcome::Nothing,
    }
}

/// Set Properties: as a file's, across the set.
pub(super) fn set_event(
    fd: &mut FormDialog,
    _page: usize,
    ev: reprisekit::controls::form::Event,
) -> Outcome {
    use reprisekit::controls::filesheet::{ADVANCED, Subject};
    use reprisekit::controls::form::Event;
    use reprisekit::controls::propsheet::{APPLY, CANCEL, OK};
    let FormKind::SetProperties(set) = &fd.kind else {
        return Outcome::Nothing;
    };
    let Some(form) = fd.pages().first() else {
        return Outcome::Nothing;
    };
    let folder_icon = set.icon_chosen.clone();
    let write = |close: bool| match set_changes(form, set) {
        (attrs, None) if attrs.is_empty() && folder_icon.is_none() && close => Outcome::Close,
        (attrs, None) if attrs.is_empty() && folder_icon.is_none() => Outcome::Nothing,
        (attrs, hidden) => Outcome::SetProperties {
            paths: set.paths.clone(),
            attrs,
            hidden,
            folder_icon: folder_icon.clone(),
            close,
            recurse: None,
        },
    };
    match ev {
        Event::Default | Event::Pressed(OK) => write(true),
        Event::Pressed(APPLY) => write(false),
        Event::Cancel | Event::Pressed(CANCEL) => Outcome::Close,
        Event::Pressed(ADVANCED) => Outcome::AdvancedAttributes {
            subject: Subject::Set {
                has_dir: set.has_dir,
            },
            robots: set.robots().map(|opened| set.adv.unwrap_or(opened)),
        },
        Event::Pressed(reprisekit::controls::filesheet::CHANGE_ICON) => Outcome::ChangeIcon {
            name: set
                .paths
                .first()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            current: set.icon_chosen.clone().flatten(),
            launcher: false,
        },
        Event::Changed(_) | Event::Redraw => {
            sync_apply(fd);
            Outcome::Redraw
        }
        Event::None | Event::Pressed(_) => Outcome::Nothing,
    }
}

/// The folder icon Change Icon left, when it differs from the folder's
/// own.
fn icon_move(base: &SheetBase) -> Option<Option<String>> {
    base.icon_chosen.clone().filter(|c| *c != base.own_icon)
}

/// The shortcut keys OK and Apply would write: the Shortcut or Web
/// Document fields that moved off what the file said, then the icon
/// Change Icon left.
fn launcher_changes(
    pages: &[reprisekit::controls::form::Form],
    base: &SheetBase,
) -> Vec<(reprisefs::launcher::Key, String)> {
    use reprisekit::controls::filesheet::{shortcut_moves, shortcut_of};
    match (base.launcher.as_ref(), shortcut_of(pages)) {
        (Some(l), Some(page)) => l.edits(&shortcut_moves(page, &l.fields()), icon_move(base)),
        _ => Vec::new(),
    }
}

/// What a shortcut's Shortcut page shows: its name, and what it runs or
/// points at, read from `l`, its keys.
pub fn shortcut_facts(
    name: &str,
    l: &reprisefs::launcher::Launcher,
) -> reprisekit::controls::filesheet::ShortcutFacts {
    reprisekit::controls::filesheet::ShortcutFacts {
        name: name.to_string(),
        target_type: l.target_type.clone(),
        target_location: l.location.clone(),
        symlink: l.kind == reprisefs::launcher::Kind::Symlink,
        fields: l.fields(),
    }
}

/// What the sheet over `info` is about, for its tabs.
pub(super) fn sheet_over(
    info: &reprisefs::props::FileInfo,
) -> reprisekit::controls::filesheet::Over {
    use reprisefs::launcher::Kind;
    use reprisekit::controls::filesheet::Over;
    match &info.launcher {
        _ if info.is_dir => Over::Folder,
        Some(l) => match l.kind {
            Kind::Program => Over::Launcher,
            Kind::Symlink => Over::Symlink { runs: l.runs },
            Kind::Web => Over::WebLink,
        },
        None => Over::File {
            shortcut: info.is_shortcut,
        },
    }
}

/// Find Target...: the program, or what the symlink points at, as the
/// Target field names it, for a window on its folder with it selected,
/// or an error box when nothing is found there.
fn find_target(pages: &[reprisekit::controls::form::Form], base: &SheetBase) -> Outcome {
    use reprisekit::controls::filesheet::{shortcut_of, shortcut_values};
    let (Some(page), Some(l)) = (shortcut_of(pages), base.launcher.as_ref()) else {
        return Outcome::Nothing;
    };
    let [target, ..] = shortcut_values(page);
    match l.find(&target, &base.path) {
        Some(found) => Outcome::FindTarget(found),
        None => Outcome::Notice(Box::new(Dialog::error(
            "Problem with Shortcut",
            l.not_found_text(&target),
        ))),
    }
}

/// The Summary fields that moved off what the file said, when the sheet
/// has a live Summary page.
fn summary_changes(
    pages: &[reprisekit::controls::form::Form],
    base: &SheetBase,
) -> Vec<(usize, String)> {
    use reprisekit::controls::filesheet::{summary_moves, summary_of};
    match (base.summary.as_ref(), summary_of(pages)) {
        (Some(opened), Some(page)) => summary_moves(page, opened),
        _ => Vec::new(),
    }
}

/// The archive and index moves Advanced Attributes' OK handed a sheet:
/// each flag its `chosen` boxes moved off `opened`, the states read.
fn robots_moves(
    opened: Option<[CheckState; 2]>,
    chosen: Option<[CheckState; 2]>,
) -> [Option<bool>; 2] {
    match (opened, chosen) {
        (Some(o), Some(c)) => reprisekit::controls::filesheet::robots_moves(o, c),
        _ => [None, None],
    }
}

/// What OK and Apply would write across a set from its page `form`: the
/// Read-only bit and the archive and index flags, and the Hidden state,
/// each when it moved off where it started to a settled state.
fn set_changes(
    form: &reprisekit::controls::form::Form,
    set: &SetBase,
) -> (reprisefs::props::AttrMoves, Option<bool>) {
    use reprisekit::controls::filesheet::{set_moves, shared_state};
    let (read_only, hidden) = set_moves(
        form,
        (shared_state(set.read_only), shared_state(set.hidden)),
    );
    let [backup, index] = robots_moves(set.robots(), set.adv);
    let attrs = reprisefs::props::AttrMoves {
        read_only,
        backup,
        index,
        executable: None,
    };
    (attrs, hidden)
}

/// What OK and Apply would write from sheet page `form`: the new name when the field or Hidden
/// changed it, attributes whose boxes moved to a settled state, and Hidden when its box moved. The
/// typed name is kept as is (spaces survive; all spaces renames nothing). Hidden dots the name
/// unless the folder's `.hidden` lists it.
pub(super) fn props_changes(
    form: &reprisekit::controls::form::Form,
    base: &SheetBase,
) -> (Option<String>, reprisefs::props::AttrMoves, Option<bool>) {
    use reprisekit::controls::filesheet;
    let typed = filesheet::typed_name(form);
    let hidden = filesheet::hidden(form);
    let dotted = base
        .path
        .file_name()
        .is_some_and(|n| n.as_encoded_bytes().starts_with(b"."));
    let dot = reprisefs::hidden::wants_dot(hidden, dotted, base.listed);
    let name = filesheet::wanted_name(typed, dot, &base.ext);
    let same = base
        .path
        .file_name()
        .is_some_and(|n| n.to_string_lossy() == name);
    let rename = (!typed.trim().is_empty() && !same && !name.contains('/')).then_some(name);
    let read_only = match filesheet::read_only(form) {
        CheckState::Mixed => None,
        state if state == base.read_only => None,
        state => Some(state.is_on()),
    };
    let [backup, index] = robots_moves(base.robots, base.adv);
    let executable = filesheet::executable(form);
    let attrs = reprisefs::props::AttrMoves {
        read_only,
        backup,
        index,
        executable: (executable != base.executable).then_some(executable),
    };
    (rename, attrs, (hidden != base.hidden).then_some(hidden))
}

/// Light a sheet's Apply, on every page, exactly while its General page
/// (or Advanced Attributes' boxes) holds a change.
fn sync_apply(fd: &mut FormDialog) {
    let Some(form) = fd.body.pages().first() else {
        return;
    };
    let dirty = match &fd.kind {
        FormKind::Properties(base) => {
            let (name, attrs, hidden) = props_changes(form, base);
            name.is_some()
                || !attrs.is_empty()
                || hidden.is_some()
                || !summary_changes(fd.body.pages(), base).is_empty()
                || icon_move(base).is_some()
                || !launcher_changes(fd.body.pages(), base).is_empty()
        }
        FormKind::SetProperties(set) => {
            let (attrs, hidden) = set_changes(form, set);
            hidden.is_some() || !attrs.is_empty() || set.icon_chosen.is_some()
        }
        _ => return,
    };
    if let Some(sheet) = fd.sheet_mut() {
        sheet.set_apply_enabled(dirty);
    }
}

/// What a Properties sheet opened on, so OK and Apply know what changed:
/// the file, the extension its name field leaves out, where Read-only
/// started, and the Opens with program's icon, which the host resolves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SheetBase {
    pub path: std::path::PathBuf,
    /// The name the sheet was opened under.
    pub name: String,
    pub ext: String,
    /// The file's read-only bit, or indeterminate on a folder.
    pub read_only: CheckState,
    /// The Executable box as it opened (unticked on a folder).
    pub executable: bool,
    pub hidden: bool,
    pub listed: bool,
    pub is_dir: bool,
    pub opens_icon: Option<String>,
    /// Advanced Attributes' archive and index boxes as read from the
    /// file, or None where its file system keeps no such flags.
    pub robots: Option<[CheckState; 2]>,
    /// Where Advanced Attributes' OK left those boxes, until the sheet
    /// writes them.
    pub adv: Option<[CheckState; 2]>,
    pub summary: Option<[String; 6]>,
    /// A folder's or a launcher's own icon as it opened (None: the
    /// standard folder, or no icon of the launcher's own), and the one
    /// Change Icon left until the sheet writes it.
    pub own_icon: Option<String>,
    pub icon_chosen: Option<Option<String>>,
    /// A shortcut's keys as the file had them (a program launcher's, a
    /// symlink's, a web link's), None for anything else.
    pub launcher: Option<reprisefs::launcher::Launcher>,
    /// A shortcut (a launcher, a web link, a symlink): the header icon
    /// wears the shortcut arrow.
    pub shortcut: bool,
}

/// What an OK or Apply wrote to one item: where it is now, and each
/// field that landed as it was written. A field whose write failed is
/// left out, so the sheet keeps it as a change.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Written {
    pub path: std::path::PathBuf,
    pub attrs: reprisefs::props::AttrMoves,
    pub hidden: Option<bool>,
    pub summary: Vec<(usize, String)>,
    pub folder_icon: Option<Option<String>>,
    pub launcher: Vec<(reprisefs::launcher::Key, String)>,
    /// The shortcut read back after its keys were written, for [`Dialog::sheet_reread`].
    pub reread: Option<reprisefs::props::FileInfo>,
}

/// A set's Properties sheet: what its items share, and where Advanced
/// Attributes' OK left the archive and index boxes until the sheet
/// writes them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetBase {
    pub set: reprisefs::props::SetInfo,
    pub adv: Option<[CheckState; 2]>,
    pub icon_chosen: Option<Option<String>>,
}

impl SetBase {
    pub fn robots(&self) -> Option<[CheckState; 2]> {
        reprisekit::controls::filesheet::robots_states(self.set.robots)
    }
}

impl std::ops::Deref for SetBase {
    type Target = reprisefs::props::SetInfo;
    fn deref(&self) -> &reprisefs::props::SetInfo {
        &self.set
    }
}

impl std::ops::DerefMut for SetBase {
    fn deref_mut(&mut self) -> &mut reprisefs::props::SetInfo {
        &mut self.set
    }
}

impl Dialog {
    pub fn sheet_path(&self) -> Option<&std::path::Path> {
        match &self.form()?.kind {
            FormKind::Properties(b) => Some(b.path.as_path()),
            _ => None,
        }
    }

    pub fn sheet_name(&self) -> Option<&str> {
        match &self.form()?.kind {
            FormKind::Properties(b) => Some(&b.name),
            _ => None,
        }
    }

    /// Put a folder walk's `t` on the sheet's Size, Size on disk and
    /// Contains rows; a set's sheet adds it to what its items hold and
    /// shows the sums with the count. False when the sheet has none of
    /// them.
    pub fn set_folder_totals(&mut self, t: reprisefs::props::FolderTotals) -> bool {
        use reprisefs::props::{format_contains, format_size};
        if let Some(f) = self.form_mut()
            && let (FormKind::SetProperties(set), Some(form)) =
                (&mut f.kind, f.body.pages_mut().first_mut())
        {
            set.add_totals(t);
            form.set_text(reprisekit::controls::filesheet::COUNT, &set.count_text());
            form.set_text("v_size", &format_size(set.size));
            form.set_text("v_disk", &format_size(set.on_disk));
            return true;
        }
        let Some((_, form)) = self.props_mut() else {
            return false;
        };
        let mut set = false;
        for (id, value) in [
            ("v_size", format_size(t.size)),
            ("v_disk", format_size(t.on_disk)),
            ("v_has", format_contains(t.files, t.folders)),
        ] {
            if form.item(id).is_some() {
                form.set_text(id, &value);
                set = true;
            }
        }
        set
    }

    /// Put `name` (with its `icon`) on the sheet's Opens with row, after
    /// the chooser behind Change... changed the file's default program.
    pub fn set_opens_with(&mut self, name: String, icon: Option<String>) -> bool {
        let Some((base, form)) = self.props_mut() else {
            return false;
        };
        if form.item("v_opens").is_none() {
            return false;
        }
        form.set_text("v_opens", &name);
        base.opens_icon = icon;
        true
    }
}
