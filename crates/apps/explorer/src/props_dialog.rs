//! The Properties sheet: General page, Advanced Attributes, Confirm Attribute Changes and the
//! multi-selection sheet. Pages and boxes are [`reprisekit::controls::filesheet`]; this side words
//! [`reprisefs::props::glance`]'s facts and keeps what Apply compares against.

use repriseapp::WinId;
use reprisefs::props::{
    AttrMoves, FileInfo, FolderTotals, SetInfo, format_contains, format_size, format_time,
};
use reprisefs::trash::Trashed;
use reprisekit::classic::widgets::CheckState;
use reprisekit::controls::filesheet;
pub use reprisekit::controls::filesheet::{
    ADV_CANCEL, ADV_OK, ADVANCED, ATTR_CANCEL, ATTR_OK, ATTR_SIZE, BIN_SHEET_H, CALCULATING,
    CHANGE, Facts, ICON_PX, READ_ONLY, RESTORE, Row, SET_ICON, SHEET_W, attr_tree_picked, bin_page,
    general_page,
};
use reprisekit::controls::form::{Control, Form};
use reprisekit::controls::propsheet::PropSheet;
use reprisekit::text::TextRenderer;
use reprisekit::theme::TextUnits;
use std::path::{Path, PathBuf};

/// The sheet's own window; modeless, its boxes come up modal over it in the [`DIALOG`] child.
pub const PROPS: WinId = "props";

pub fn facts(info: &FileInfo, is_12h: bool) -> Facts {
    let walking = info.is_dir && info.contains.is_none();
    let total = |v: String| if walking { CALCULATING.to_string() } else { v };
    Facts {
        is_dir: info.is_dir,
        type_name: info.type_name.clone(),
        opens_with: info.opens_with.clone(),
        description: info.description.clone(),
        location: info.location.clone(),
        size: total(format_size(info.size)),
        on_disk: total(format_size(info.on_disk)),
        contains: match info.contains {
            Some((f, d)) => Some(format_contains(f, d)),
            None => walking.then(|| CALCULATING.to_string()),
        },
        created: info.created.map(|t| format_time(t, is_12h)),
        modified: format_time(info.modified, is_12h),
        accessed: format_time(info.accessed, is_12h),
        read_only: info.read_only,
        hidden: info.hidden,
    }
}

pub fn over(info: &FileInfo) -> filesheet::Over {
    use reprisefs::launcher::Kind;
    match &info.launcher {
        _ if info.is_dir => filesheet::Over::Folder,
        Some(l) => match l.kind {
            Kind::Program => filesheet::Over::Launcher,
            Kind::Symlink => filesheet::Over::Symlink { runs: l.runs },
            Kind::Web => filesheet::Over::WebLink,
        },
        None => filesheet::Over::File {
            shortcut: info.is_shortcut,
        },
    }
}

pub fn rows(info: &FileInfo, is_12h: bool) -> Vec<Row> {
    filesheet::rows(&facts(info, is_12h))
}

pub fn set_facts(set: &SetInfo) -> filesheet::SetFacts {
    let total = |v: String| {
        if set.walking {
            CALCULATING.to_string()
        } else {
            v
        }
    };
    filesheet::SetFacts {
        has_dir: set.has_dir,
        count: total(set.count_text()),
        type_text: set.type_text(),
        location: set.location_text(),
        size: total(format_size(set.size)),
        on_disk: total(format_size(set.on_disk)),
        read_only: filesheet::shared_state(set.read_only),
        hidden: filesheet::shared_state(set.hidden),
    }
}

pub fn attr_form(
    attrs: &AttrMoves,
    hidden: Option<bool>,
    subject: filesheet::Subject,
    text: &TextRenderer,
    px: u32,
    u: TextUnits,
) -> Form {
    filesheet::attr_form(&attrs.verbs(hidden), subject, text, px, u)
}

pub struct Props {
    pub info: FileInfo,
    pub sheet: PropSheet,
    pub opened_name: String,
    pub opened_read_only: CheckState,
    /// What the caption calls the thing; a shell folder is titled by its shell name while the field
    /// keeps the real name.
    pub caption: String,
    /// Over an item in the Recycle Bin: [`filesheet::bin_page`], read-only but Restore. Closes once
    /// the item leaves the listing.
    pub restore: Option<crate::fileops::BinItem>,
    /// Over several items: the set's page ([`filesheet::set_rows`]); nothing renames, OK and Apply
    /// write Read-only and Hidden across the set.
    pub set: Option<SetInfo>,
    pub robots: Option<[CheckState; 2]>,
    pub adv: Option<[CheckState; 2]>,
    pub summary: Option<[String; 6]>,
    pub own_icon: Option<String>,
    pub icon_chosen: Option<Option<String>>,
}

pub fn bin_facts(info: &FileInfo, t: Option<&Trashed>, is_12h: bool) -> filesheet::BinFacts {
    filesheet::BinFacts {
        type_name: info.type_name.clone(),
        origin: t
            .and_then(|t| t.original.parent())
            .map(|d| crate::place::Place::from_path(d).title())
            .unwrap_or_default(),
        size: bin_size(info),
        deleted: t.and_then(|t| t.deleted).map(|s| format_time(s, is_12h)),
        created: info.created.map(|s| format_time(s, is_12h)),
        read_only: info.read_only,
        hidden: info.hidden,
    }
}

fn bin_size(info: &FileInfo) -> String {
    if info.is_dir && info.contains.is_none() {
        CALCULATING.to_string()
    } else {
        crate::window::short_size(info.size)
    }
}

impl Props {
    pub fn general(&self) -> &Form {
        &self.sheet.pages[0]
    }

    pub fn general_mut(&mut self) -> &mut Form {
        &mut self.sheet.pages[0]
    }

    pub fn name(&self) -> String {
        filesheet::typed_name(self.general()).trim().to_string()
    }

    pub fn new_name(&self) -> Option<String> {
        if self.set.is_some() {
            return None;
        }
        let name = self.name();
        let wanted = self.wanted_name(&name);
        let old = self.info.file_name();
        (!name.is_empty() && wanted != old).then_some(wanted)
    }

    pub fn wanted_name(&self, typed: &str) -> String {
        let dotted = self
            .info
            .path
            .file_name()
            .is_some_and(|n| n.as_encoded_bytes().starts_with(b"."));
        let hidden = filesheet::hidden(self.general());
        let dot = reprisefs::hidden::wants_dot(hidden, dotted, self.info.listed);
        filesheet::wanted_name(typed, dot, &self.info.hidden_ext)
    }

    pub fn hidden(&self) -> Option<bool> {
        if self.set.is_some() {
            return self.set_moves().1;
        }
        let now = filesheet::hidden(self.general());
        (now != self.info.hidden).then_some(now)
    }

    pub fn read_only(&self) -> Option<bool> {
        if self.set.is_some() {
            return self.set_moves().0;
        }
        match filesheet::read_only(self.general()) {
            CheckState::Mixed => None,
            state if state == self.opened_read_only => None,
            state => Some(state.is_on()),
        }
    }

    fn set_moves(&self) -> (Option<bool>, Option<bool>) {
        match &self.set {
            Some(set) => filesheet::set_moves(
                self.general(),
                (
                    filesheet::shared_state(set.read_only),
                    filesheet::shared_state(set.hidden),
                ),
            ),
            None => (None, None),
        }
    }

    pub fn dirty(&self) -> bool {
        self.new_name().is_some()
            || !self.attrs().is_empty()
            || self.hidden().is_some()
            || !self.summary_moves().is_empty()
            || self.icon_move().is_some()
            || !self.launcher_edits().is_empty()
    }

    pub fn icon_move(&self) -> Option<Option<String>> {
        self.icon_chosen.clone().filter(|c| *c != self.own_icon)
    }

    pub fn folder_icon_move(&self) -> Option<Option<String>> {
        self.icon_move().filter(|_| self.info.launcher.is_none())
    }

    pub fn launcher_edits(&self) -> Vec<(reprisefs::launcher::Key, String)> {
        match (
            &self.info.launcher,
            filesheet::shortcut_of(&self.sheet.pages),
        ) {
            (Some(l), Some(page)) => l.edits(
                &filesheet::shortcut_moves(page, &l.fields()),
                self.icon_move(),
            ),
            _ => Vec::new(),
        }
    }

    pub fn shown_icon(&self) -> String {
        self.icon_chosen
            .clone()
            .unwrap_or_else(|| self.own_icon.clone())
            .unwrap_or_else(|| filesheet::STANDARD_FOLDER_ICON.to_string())
    }

    pub fn summary_moves(&self) -> Vec<(usize, String)> {
        match (&self.summary, filesheet::summary_of(&self.sheet.pages)) {
            (Some(opened), Some(page)) => filesheet::summary_moves(page, opened),
            _ => Vec::new(),
        }
    }

    pub fn attrs(&self) -> AttrMoves {
        let [backup, index] = match (self.robots, self.adv) {
            (Some(o), Some(c)) => filesheet::robots_moves(o, c),
            _ => [None, None],
        };
        AttrMoves {
            read_only: self.read_only(),
            backup,
            index,
        }
    }

    pub fn robots_shown(&self) -> Option<[CheckState; 2]> {
        self.robots.map(|read| self.adv.unwrap_or(read))
    }

    pub fn title(&self) -> String {
        format!("{} Properties", self.caption)
    }

    pub fn size(&self) -> (i32, i32) {
        self.sheet.size
    }

    pub fn set_totals(&mut self, t: FolderTotals) {
        if let Some(set) = self.set.as_mut() {
            set.add_totals(t);
            let facts = set_facts(set);
            let page = self.general_mut();
            for (id, value) in [
                (filesheet::COUNT, facts.count),
                ("v_size", facts.size),
                ("v_disk", facts.on_disk),
            ] {
                if let Some(it) = page.item_mut(id)
                    && let Control::Label { text, .. } = &mut it.control
                {
                    *text = value;
                }
            }
            return;
        }
        self.info.add_totals(t);
        let size = if self.restore.is_some() {
            bin_size(&self.info)
        } else {
            format_size(t.size)
        };
        let page = self.general_mut();
        for (id, value) in [
            ("v_size", size),
            ("v_disk", format_size(t.on_disk)),
            ("v_has", format_contains(t.files, t.folders)),
        ] {
            if let Some(it) = page.item_mut(id)
                && let Control::Label { text, .. } = &mut it.control
            {
                *text = value;
            }
        }
    }
}

pub struct AttrConfirm {
    pub set: Option<Vec<PathBuf>>,
    pub new_name: Option<String>,
    pub attrs: AttrMoves,
    pub hidden: Option<bool>,
    pub close: bool,
    pub form: Form,
}

pub(crate) fn folder_has_contents(path: &Path) -> bool {
    std::fs::read_dir(path).is_ok_and(|mut d| d.next().is_some())
}

#[cfg(test)]
#[path = "props_tests.rs"]
mod tests;
