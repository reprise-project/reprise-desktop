use std::path::PathBuf;

use reprisefs::apps::AppEntry;
use reprisekit::startpanel::{ResultGroup, ResultRow};

use crate::compositor::State;
use crate::raster::Canvas;
use crate::search::{
    CALC_ICON, SearchOwner, Setting, calc_row, file_row, rank_settings, word_starts,
};

use super::RESULT_ICON;
use super::lists::{app_icon, file_name, one_per_name, unread};

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
    Files,
}

pub(super) fn result_groups(
    calc: Option<&str>,
    apps: &[String],
    settings: &[String],
    files: &[PathBuf],
) -> (Vec<ResultGroup>, Vec<ResultKind>) {
    let mut groups = Vec::new();
    let mut kinds = Vec::new();
    let mut group = |kind, title: String, rows: Vec<String>| {
        if !rows.is_empty() {
            groups.push(ResultGroup {
                title,
                rows: rows
                    .into_iter()
                    .map(|name| ResultRow { name, icon: None })
                    .collect(),
            });
            kinds.push(kind);
        }
    };
    group(
        ResultKind::Calc,
        "Calculator".into(),
        calc.map(str::to_string).into_iter().collect(),
    );
    let apps = &apps[..apps.len().min(SEARCH_ROWS)];
    group(
        ResultKind::Programs,
        format!("Programs ({})", apps.len()),
        apps.to_vec(),
    );
    group(
        ResultKind::Settings,
        format!("Settings ({})", settings.len()),
        settings.to_vec(),
    );
    let files = &files[..files.len().min(SEARCH_ROWS)];
    group(
        ResultKind::Files,
        format!("Files ({})", files.len()),
        files.iter().map(|p| file_name(p)).collect(),
    );
    (groups, kinds)
}

impl State {
    pub(super) fn start_panel_query_changed(&mut self) {
        let Some(v) = self.start_panel.as_ref() else {
            return;
        };
        let query = v.model.query().to_string();
        let kept = self.file_query_changed(SearchOwner::StartPanel, &query, SEARCH_ROWS);
        let State {
            start_panel,
            app_index,
            ..
        } = self;
        if let Some(v) = start_panel.as_mut() {
            let listed = one_per_name(app_index);
            v.result_apps = panel_matches(&query, app_index, &listed, SEARCH_ROWS);
            v.result_settings = rank_settings(&query, app_index, &v.result_apps, SEARCH_SETTINGS);
            v.answer = calc_row(&query).map(|(_, answer)| answer);
            v.search.files = kept;
        }
        self.start_panel_show_results();
    }

    pub(crate) fn start_panel_files(&mut self, paths: Vec<PathBuf>) {
        if let Some(v) = self.start_panel.as_mut() {
            v.search.files = paths;
        }
        self.start_panel_show_results();
        self.redraw_start_panel();
    }

    pub(super) fn start_panel_show_results(&mut self) {
        crate::tooltip::hover_start_panel(self, None);
        let Some(scale) = self.start_panel_scale() else {
            return;
        };
        let px = RESULT_ICON * scale;
        let home = std::env::var_os("HOME").map(PathBuf::from);
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
        let names: Vec<String> = v
            .result_apps
            .iter()
            .map(|&i| app_index[i].name.clone())
            .collect();
        let settings: Vec<String> = v
            .result_settings
            .iter()
            .map(|s| s.name(app_index))
            .collect();
        let calc = v.answer.as_deref().map(|a| format!("= {a}"));
        let (mut groups, kinds) =
            result_groups(calc.as_deref(), &names, &settings, &v.search.files);
        v.result_kinds = kinds;
        let mut icon_rows: Vec<Option<Canvas>> = Vec::new();
        if calc.is_some() {
            icon_rows.push(icons.get(CALC_ICON, px));
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
            icon_rows.extend(v.search.files.iter().map(|p| {
                let is_dir = v.search.is_dir(p);
                file_row(p, is_dir, home.as_deref(), mime, icons, px).icon
            }));
        });
        let rows = groups.iter_mut().flat_map(|g| g.rows.iter_mut());
        for (row, icon) in rows.zip(icon_rows) {
            row.icon = icon;
        }
        v.model.set_results(groups);
    }

    pub(super) fn start_panel_run_result(&mut self, g: usize, r: usize) {
        let Some(v) = self.start_panel.as_ref() else {
            return;
        };
        enum Run {
            Copy(String),
            App(Box<AppEntry>),
            Setting(Setting),
            File(std::path::PathBuf, crate::desktop::FileFacts),
        }
        let run = match v.result_kinds.get(g) {
            Some(ResultKind::Calc) => v.answer.clone().map(Run::Copy),
            Some(ResultKind::Programs) => v
                .result_apps
                .get(r)
                .and_then(|&a| self.app_index.get(a))
                .map(|e| Run::App(Box::new(e.clone()))),
            Some(ResultKind::Settings) => v.result_settings.get(r).copied().map(Run::Setting),
            Some(ResultKind::Files) => v
                .search
                .files
                .get(r)
                .map(|p| Run::File(p.clone(), v.search.facts_of(p))),
            None => None,
        };
        self.close_start_panel();
        match run {
            Some(Run::Copy(answer)) => self.clip_copy(&answer),
            Some(Run::App(entry)) => self.launch_app(&entry),
            Some(Run::Setting(s)) => self.open_setting(s),
            Some(Run::File(path, facts)) => self.open_found(&path, &facts),
            None => {}
        }
    }
}
