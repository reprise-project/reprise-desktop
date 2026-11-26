//! The Window Rules page's buttons and its rule dialog. The dialog offers the open windows' app
//! ids, asked of the compositor off the loop each time it opens.

use crate::registry::Page;
use crate::windowrules::Rule;
use crate::{SettingsApp, ruledlg, rulespage, take};
use repriseapp::keys::form_input;
use repriseapp::{Ctx, Event, Reply};

impl SettingsApp {
    pub(crate) fn rules_button(&mut self, id: &str) -> Reply {
        let page = &mut self.sheet.pages[Page::WindowRules.index()];
        match id {
            rulespage::ADD => return self.open_rule(None),
            rulespage::EDIT => {
                return match rulespage::picked(page) {
                    Some(row) => self.open_rule(Some(row)),
                    None => Reply::none(),
                };
            }
            _ => {}
        }
        let rules = &mut self.want.window_rules;
        let done = match id {
            rulespage::REMOVE => rulespage::remove(page, rules),
            rulespage::UP => rulespage::shift(page, rules, -1),
            rulespage::DOWN => rulespage::shift(page, rules, 1),
            _ => false,
        };
        if done {
            self.sync_apply();
        }
        Reply::redraw()
    }

    /// The list's pick moved: the buttons follow it.
    pub(crate) fn rules_picked(&mut self) -> Reply {
        let page = &mut self.sheet.pages[Page::WindowRules.index()];
        rulespage::fill(page, &self.want.window_rules, None);
        Reply::redraw()
    }

    /// Open the dialog on rule `row`, or for a new rule when `None`.
    fn open_rule(&mut self, row: Option<usize>) -> Reply {
        let rule = row.and_then(|i| self.want.window_rules.get(i));
        if row.is_some() && rule.is_none() {
            return Reply::none();
        }
        let on = row.zip(rule.cloned());
        let outputs = ruledlg::screens(self.display.model.heads());
        self.rule_dlg = Some((ruledlg::RuleDialog::new(rule, outputs), on));
        // Tests leave the session bus alone.
        if !cfg!(test) {
            self.open_windows = Some(self.spawn(|| {
                let found = reprisebus::tray::windows_by_app().unwrap_or_else(|e| {
                    log::warn!("open windows: {e}");
                    Vec::new()
                });
                found.into_iter().map(|(app_id, _)| app_id).collect()
            }));
        }
        Reply::redraw_all()
    }

    /// The compositor listed the open windows; the dialog offers their programs.
    pub(crate) fn open_windows_done(&mut self) -> bool {
        let Some(rx) = &self.open_windows else {
            return false;
        };
        let ids = match take(rx) {
            Ok(None) => return false,
            Ok(Some(ids)) => ids,
            Err(()) => Vec::new(),
        };
        self.open_windows = None;
        match self.rule_dlg.as_mut() {
            Some((d, _)) => {
                d.offer_windows(ids);
                true
            }
            None => false,
        }
    }

    pub(crate) fn rule_dlg_event(&mut self, ev: Event, ctx: &Ctx) -> Reply {
        let out = {
            let Some((d, _)) = self.rule_dlg.as_mut() else {
                return Reply::none();
            };
            match ev {
                Event::CloseRequest => ruledlg::Outcome::Cancel,
                Event::Blink => {
                    d.form.blink(ctx.caret_on);
                    return Reply::redraw();
                }
                ev => match form_input(&mut d.form, &ev, ctx) {
                    Some(ev) => d.event(ev),
                    None => return Reply::none(),
                },
            }
        };
        match out {
            ruledlg::Outcome::None => Reply::none(),
            ruledlg::Outcome::Redraw => Reply::redraw(),
            ruledlg::Outcome::Cancel => {
                self.rule_dlg = None;
                self.open_windows = None;
                Reply::redraw_all()
            }
            ruledlg::Outcome::Accept(rule) => {
                let on = self.rule_dlg.take().and_then(|(_, on)| on);
                self.open_windows = None;
                let rules = &mut self.want.window_rules;
                let at = (on.as_ref()).and_then(|(i, was)| edited_row(rules, *i, was));
                let at = match at {
                    Some(i) => {
                        rules[i] = rule;
                        i
                    }
                    None if on.is_some_and(|(_, was)| was == rule) => {
                        return Reply::redraw_all();
                    }
                    None => {
                        rules.push(rule);
                        rules.len() - 1
                    }
                };
                let page = &mut self.sheet.pages[Page::WindowRules.index()];
                rulespage::fill(page, rules, Some(at));
                self.sync_apply();
                Reply::redraw_all()
            }
        }
    }
}

/// Where `was`, the rule the dialog opened on at row `i`, is in `rules` now that the file may have
/// moved it: row `i` when it still holds it, else the first row equal to it. `None` when it went:
/// an edited dialog's rule joins the list as a new one, an unedited one is dropped.
fn edited_row(rules: &[Rule], i: usize, was: &Rule) -> Option<usize> {
    match rules.get(i) {
        Some(r) if r == was => Some(i),
        _ => rules.iter().position(|r| r == was),
    }
}
