use std::path::Path;

use repriseconf::search::word_starts;
use reprisefs::apps::AppEntry;
use reprisekit::startpanel::{ResultGroup, ResultRow, SMALL_ICON};

use crate::compositor::State;
use crate::raster::Canvas;
use crate::search::{Found, Picked, SearchOwner, calc_icon, calc_row, rank_settings};

use super::lists::{app_icon, one_per_name, unread};

const SEARCH_ROWS: usize = 8;
const SEARCH_SETTINGS: usize = 4;

pub(super) fn panel_matches(
    query: &str,
    apps: &[AppEntry],
    kept: &[bool],
    max: usize,
) -> Vec<usize> {
    reprisekit::fuzzy::rank(query, apps.iter().map(|a| a.name.as_str()))
        .into_iter()
        .filter(|&i| kept[i] && word_starts(query, &apps[i].name))
        .take(max)
        .collect()
}

fn with_mime<R>(
    desktop: Option<&crate::desktopview::DesktopView>,
    f: impl FnOnce(&reprisefs::apps::MimeDb) -> R,
) -> R {
    match desktop {
        Some(d) => f(&d.mime),
        None => f(&reprisefs::apps::MimeDb::empty()),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ResultKind {
    Calc,
    Programs,
    Settings,
    /// Found section `.0` of the search ([`crate::search::FileSearch::sections`]).
    Found(usize),
}

/// The search's groups and what each holds: the answer, programs, settings, then each found
/// section under its heading, every group but the answer counted. A setting is (name, detail),
/// the detail [`crate::search::Setting::detail`]'s or empty. A found row's detail is its folder
/// (`~` for `home`) or its search provider's description.
pub(super) fn result_groups<'a>(
    calc: Option<&str>,
    apps: &[String],
    settings: &[(String, String)],
    found: impl Iterator<Item = (&'a str, &'a [Found])>,
    home: Option<&Path>,
) -> (Vec<ResultGroup>, Vec<ResultKind>) {
    let mut groups = Vec::new();
    let mut kinds = Vec::new();
    let mut group = |kind, title: String, rows: Vec<ResultRow>| {
        if !rows.is_empty() {
            groups.push(ResultGroup { title, rows });
            kinds.push(kind);
        }
    };
    let named = |names: &[String]| -> Vec<ResultRow> {
        (names.iter())
            .map(|name| ResultRow {
                name: name.clone(),
                ..ResultRow::default()
            })
            .collect()
    };
    let calc: Vec<String> = calc.map(str::to_string).into_iter().collect();
    group(ResultKind::Calc, "Calculator".into(), named(&calc));
    let apps = &apps[..apps.len().min(SEARCH_ROWS)];
    group(
        ResultKind::Programs,
        format!("Programs ({})", apps.len()),
        named(apps),
    );
    group(
        ResultKind::Settings,
        format!("Settings ({})", settings.len()),
        (settings.iter())
            .map(|(name, detail)| ResultRow {
                name: name.clone(),
                icon: None,
                detail: detail.clone(),
            })
            .collect(),
    );
    for (s, (heading, rows)) in found.enumerate() {
        let rows = &rows[..rows.len().min(SEARCH_ROWS)];
        let rows: Vec<ResultRow> = (rows.iter())
            .map(|f| ResultRow {
                name: f.name(),
                icon: None,
                detail: f.detail(home),
            })
            .collect();
        group(
            ResultKind::Found(s),
            format!("{heading} ({})", rows.len()),
            rows,
        );
    }
    (groups, kinds)
}

impl State {
    pub(super) fn start_panel_query_changed(&mut self) {
        let Some(v) = self.start_panel.as_ref() else {
            return;
        };
        let query = v.model.query().to_string();
        self.file_query_changed(SearchOwner::StartPanel, &query, SEARCH_ROWS);
        let State {
            start_panel,
            app_index,
            ..
        } = self;
        if let Some(v) = start_panel.as_mut() {
            let listed = one_per_name(app_index);
            v.result_apps = panel_matches(&query, app_index, &listed, SEARCH_ROWS);
            v.result_settings = rank_settings(&query, app_index, &v.result_apps, SEARCH_SETTINGS);
            v.answer = calc_row(&query);
        }
        self.start_panel_show_results();
    }

    pub(crate) fn start_panel_found(&mut self) {
        self.start_panel_show_results();
        self.redraw_start_panel();
    }

    pub(super) fn start_panel_show_results(&mut self) {
        crate::tooltip::hover_start_panel(self, None);
        let Some(scale) = self.start_panel_scale() else {
            return;
        };
        let State {
            start_panel,
            app_index,
            icons,
            desktop,
            ..
        } = self;
        let Some(v) = start_panel.as_mut() else {
            return;
        };
        let px = v.model.icon_px(SMALL_ICON, scale);
        let names: Vec<String> = v
            .result_apps
            .iter()
            .map(|&i| app_index[i].name.clone())
            .collect();
        let query = v.model.query();
        let settings: Vec<(String, String)> = (v.result_settings.iter())
            .map(|s| {
                let detail = s.detail(query, app_index).unwrap_or_default();
                (s.name(app_index), detail.to_string())
            })
            .collect();
        let calc = v.answer.as_ref().map(|(row, _)| row.clone());
        let found = (v.search.sections()).map(|(source, rows)| (source.heading(), rows));
        let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
        let (mut groups, kinds) =
            result_groups(calc.as_deref(), &names, &settings, found, home.as_deref());
        v.result_kinds = kinds;
        let mut icon_rows: Vec<Option<Canvas>> = Vec::new();
        if calc.is_some() {
            icon_rows.push(calc_icon(icons, px));
        }
        icon_rows.extend(
            v.result_apps
                .iter()
                .map(|&i| app_icon(icons, &app_index[i], px)),
        );
        icon_rows.extend(
            v.result_settings
                .iter()
                .map(|s| s.icon(app_index).and_then(|n| unread(icons, &n, px))),
        );
        with_mime(desktop.as_ref(), |mime| {
            for (source, rows) in v.search.sections() {
                let rows = &rows[..rows.len().min(SEARCH_ROWS)];
                icon_rows.extend(
                    rows.iter()
                        .map(|f| v.search.icon(source, f, mime, icons, px)),
                );
            }
        });
        let rows = groups.iter_mut().flat_map(|g| g.rows.iter_mut());
        for (row, icon) in rows.zip(icon_rows) {
            row.icon = icon;
        }
        v.model.set_results(groups);
    }

    /// Row `row` of found section `section`, as Enter on it would.
    pub(crate) fn start_panel_open_found(&mut self, section: usize, row: usize) {
        let Some(v) = self.start_panel.as_ref() else {
            return;
        };
        let found = ResultKind::Found(section);
        if let Some(g) = v.result_kinds.iter().position(|k| *k == found) {
            self.start_panel_run_result(g, row);
        }
    }

    pub(super) fn start_panel_run_result(&mut self, g: usize, r: usize) {
        let Some(v) = self.start_panel.as_mut() else {
            return;
        };
        if let Some(&ResultKind::Found(s)) = v.result_kinds.get(g)
            && !v.search.open_now(s, r)
        {
            return;
        }
        let v = &*v;
        let picked = match v.result_kinds.get(g) {
            Some(ResultKind::Calc) => v.answer.clone().map(|(_, a)| Picked::Copy(a)),
            Some(ResultKind::Programs) => (v.result_apps.get(r))
                .and_then(|&a| self.app_index.get(a))
                .map(|e| Picked::App(Box::new(e.clone()))),
            Some(ResultKind::Settings) => v.result_settings.get(r).copied().map(Picked::Setting),
            Some(&ResultKind::Found(s)) => v.search.picked(s, r),
            None => None,
        };
        let query = v.model.query().to_string();
        self.close_start_panel();
        if let Some(picked) = picked {
            self.run_search_pick(picked, &query);
        }
    }
}
