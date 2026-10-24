//! The Advanced page's list of programs that asked for the location, and its Forget.

use crate::registry::Page;
use crate::{SettingsApp, pages, registry};
use repriseapp::Reply;

/// A remembered location answer: from Reprise's own prompt (`location.toml`), or for a Flatpak
/// program from the Location portal's permission store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LocationAnswer {
    pub(crate) id: String,
    pub(crate) allow: bool,
    pub(crate) portal: bool,
}

/// The location list's answers: the prompt's, then the portal's.
pub(crate) fn location_answers(
    agent: std::collections::BTreeMap<String, bool>,
    portal: Vec<(String, bool)>,
) -> Vec<LocationAnswer> {
    let agent = agent.into_iter().map(|(id, allow)| LocationAnswer {
        id,
        allow,
        portal: false,
    });
    let portal = portal.into_iter().map(|(id, allow)| LocationAnswer {
        id,
        allow,
        portal: true,
    });
    agent.chain(portal).collect()
}

/// The Location portal's answers; none when the permission store does not answer. Tests leave
/// the session bus alone.
fn portal_location_answers() -> Vec<(String, bool)> {
    if cfg!(test) {
        return Vec::new();
    }
    reprisebus::permissions::location_answers().unwrap_or_else(|e| {
        log::warn!("no Flatpak location answers from the permission store: {e}");
        Vec::new()
    })
}

/// The location list as read off the loop: its answers, each row's text and icon name.
#[derive(Debug, Default)]
pub(crate) struct Located {
    answers: Vec<LocationAnswer>,
    rows: Vec<(String, bool)>,
    icons: Vec<Option<String>>,
}

/// Reprise's own answers from `file`, the portal's, and the program each names.
fn read_locations(file: &std::path::Path) -> Located {
    let answers = location_answers(
        repriseconf::location::load_from(file),
        portal_location_answers(),
    );
    let entries = if answers.is_empty() {
        Vec::new()
    } else {
        reprisefs::apps::scan(&reprisefs::apps::default_dirs())
    };
    let found: Vec<_> = answers
        .iter()
        .map(|a| reprisefs::apps::entry_for_app_id(&entries, &a.id, |_| true))
        .collect();
    let rows = answers
        .iter()
        .zip(&found)
        .map(|(a, e)| (e.map_or_else(|| a.id.clone(), |e| e.name.clone()), a.allow))
        .collect();
    let icons = found
        .iter()
        .map(|e| e.and_then(|e| e.icon.clone()))
        .collect();
    Located {
        answers,
        rows,
        icons,
    }
}

impl SettingsApp {
    /// Read the location list off the loop the first time the Advanced page shows: the
    /// permission store can take seconds to answer. The list says it is loading until then.
    pub(crate) fn ensure_locations(&mut self) {
        if !self.locations_due
            || self.locating.is_some()
            || registry::open(&self.sheet) != Page::Advanced
        {
            return;
        }
        self.locations_due = false;
        let file = self.location_file.clone();
        self.locating = Some(self.spawn(move || read_locations(&file)));
        pages::show_locations_loading(&mut self.locations);
        self.sync_forget();
    }

    pub(crate) fn show_locations(&mut self, found: Located) {
        self.location_icons = found.icons;
        self.location_icons_due = true;
        self.location_answers = found.answers;
        self.locations.selection.set_only(None);
        pages::fill_locations(&mut self.locations, &found.rows);
        self.sync_forget();
    }

    pub(crate) fn sync_forget(&mut self) {
        let on = self.locating.is_none() && self.locations.selection.focus.is_some();
        self.sheet.pages[Page::Advanced.index()].set_enabled(pages::FORGET, on);
    }

    /// Forget drops the picked answer at once, from the state file or the portal's permission
    /// store; it waits for no Apply.
    pub(crate) fn forget_location(&mut self) -> Reply {
        let Some((i, a)) = (self.locations.selection.focus)
            .filter(|_| self.locating.is_none())
            .and_then(|i| Some((i, self.location_answers.get(i)?)))
        else {
            return Reply::none();
        };
        let id = &a.id;
        let forgot = if a.portal {
            reprisebus::permissions::forget_location(id).map_err(|e| e.to_string())
        } else {
            repriseconf::location::forget_in(&self.location_file, id).map_err(|e| e.to_string())
        };
        match forgot {
            Ok(()) => {
                self.location_answers.remove(i);
                self.location_icons.remove(i);
                self.locations.rows.remove(i);
                self.locations.selection.set_only(None);
                self.sync_forget();
            }
            Err(e) => {
                log::warn!("forget the location answer for {id}: {e}");
                self.fail("The location answer could not be forgotten.");
                self.locations_due = true;
                self.ensure_locations();
            }
        }
        self.sync_apply();
        Reply::redraw()
    }
}
