//! Find, Replace and the not-found box.

use super::*;

/// Find Next and the Replace buttons act only on a non-empty query.
fn sync_actions(f: &mut Form) {
    let live = !Notepad::field(f, finddlg::QUERY).is_empty();
    for id in [finddlg::FIND_NEXT, finddlg::REPLACE, finddlg::REPLACE_ALL] {
        if let Some(i) = f.index_of(id)
            && let Control::Button { enabled, .. } = &mut f.items[i].control
        {
            *enabled = live;
        }
    }
}

fn checked(f: &Form, id: &str) -> bool {
    f.checked(id).expect("a check box or radio")
}

/// Replace has no direction: it always searches down.
fn replace_query(f: &Form) -> (String, multiedit::FindOpts) {
    let match_case = checked(f, finddlg::MATCH_CASE);
    let query = Notepad::field(f, finddlg::QUERY);
    (
        query,
        multiedit::FindOpts {
            match_case,
            down: true,
        },
    )
}

impl Notepad {
    pub(crate) fn find_next(&mut self) -> bool {
        let Some((query, opts)) = self.last_query.clone() else {
            return false;
        };
        // Search onward from the selection's far end, so a repeat does
        // not keep finding the match already selected.
        let (start, end) = self.edit.selection();
        let from = if opts.down { end } else { start };
        match multiedit::find(self.edit.lines(), &query, from, &opts) {
            Some((s, e)) => {
                self.edit.place(s, false);
                self.edit.place(e, true);
                true
            }
            None => {
                self.notfound = Some(self.notfound_box(&query));
                false
            }
        }
    }

    pub(crate) fn replace_once(&mut self, query: &str, with: &str, match_case: bool) -> bool {
        let selected = self.edit.selected_text();
        let matches = if match_case {
            selected == query
        } else {
            selected.to_lowercase() == query.to_lowercase()
        };
        if matches {
            self.edit.paste(with);
            self.dirty = true;
        }
        self.find_next();
        matches
    }

    pub(crate) fn replace_every(
        &mut self,
        query: &str,
        with: &str,
        o: &multiedit::FindOpts,
    ) -> usize {
        let n = self.edit.replace_all(query, with, o);
        if n > 0 {
            self.dirty = true;
        }
        n
    }

    pub(crate) fn open_replace(&mut self) {
        self.find = None;
        let query = self
            .last_query
            .as_ref()
            .map(|(q, _)| q.clone())
            .unwrap_or_default();
        let match_case = self.last_query.as_ref().is_some_and(|(_, o)| o.match_case);
        self.replace = Some(finddlg::replace_form(&finddlg::ReplaceFields {
            query,
            with: String::new(),
            match_case,
        }));
    }

    pub(crate) fn open_find(&mut self) {
        self.replace = None;
        let query = self
            .last_query
            .as_ref()
            .map(|(q, _)| q.clone())
            .unwrap_or_default();
        let down = self
            .last_query
            .as_ref()
            .map(|(_, o)| o.down)
            .unwrap_or(true);
        let match_case = self.last_query.as_ref().is_some_and(|(_, o)| o.match_case);
        self.find = Some(finddlg::find_form(&finddlg::FindFields {
            query,
            match_case,
            down,
        }));
    }

    pub(crate) fn notfound_box(&self, query: &str) -> MsgBox {
        ok_box(tr!("not-found", query = query), self.info_icon.clone())
    }

    pub(crate) fn find_event(&mut self, ev: Event, ctx: &mut Ctx) -> Reply {
        if let Event::Key(k) = &ev
            && let Some(r) = self.accel(k, ctx)
        {
            return r;
        }
        match form_event(
            &mut self.find,
            &ev,
            &ctx.text,
            self.forms.fonts.size_px,
            ctx.caret_on,
        ) {
            FormStep::Closed => {
                self.find = None;
                Reply::redraw_all()
            }
            FormStep::Redraw => Reply::redraw(),
            FormStep::None => Reply::none(),
            FormStep::Out(out) => self.find_outcome(out, ctx),
        }
    }

    pub(crate) fn find_outcome(&mut self, out: form::Event, ctx: &Ctx) -> Reply {
        match out {
            form::Event::Cancel | form::Event::Pressed(finddlg::CANCEL) => {
                self.find = None;
                Reply::redraw_all()
            }
            form::Event::Default | form::Event::Pressed(finddlg::FIND_NEXT) => {
                if let Some(f) = &self.find {
                    let query = Notepad::field(f, finddlg::QUERY);
                    let match_case = checked(f, finddlg::MATCH_CASE);
                    let down = checked(f, finddlg::DOWN);
                    self.last_query = Some((query, multiedit::FindOpts { match_case, down }));
                }
                if self.find_next() {
                    self.scroll_to_caret(ctx);
                }
                Reply::redraw_all()
            }
            form::Event::Changed(finddlg::QUERY) => {
                if let Some(f) = &mut self.find {
                    sync_actions(f);
                }
                Reply::redraw_all()
            }
            form::Event::None => Reply::none(),
            _ => Reply::redraw(),
        }
    }

    pub(crate) fn replace_event(&mut self, ev: Event, ctx: &mut Ctx) -> Reply {
        if let Event::Key(k) = &ev
            && let Some(r) = self.accel(k, ctx)
        {
            return r;
        }
        match form_event(
            &mut self.replace,
            &ev,
            &ctx.text,
            self.forms.fonts.size_px,
            ctx.caret_on,
        ) {
            FormStep::Closed => {
                self.replace = None;
                Reply::redraw_all()
            }
            FormStep::Redraw => Reply::redraw(),
            FormStep::None => Reply::none(),
            FormStep::Out(out) => self.replace_outcome(out, ctx),
        }
    }

    pub(crate) fn replace_outcome(&mut self, out: form::Event, ctx: &Ctx) -> Reply {
        match out {
            form::Event::Cancel | form::Event::Pressed(finddlg::CANCEL) => {
                self.replace = None;
                Reply::redraw_all()
            }
            form::Event::Default | form::Event::Pressed(finddlg::FIND_NEXT) => {
                if let Some(f) = &self.replace {
                    self.last_query = Some(replace_query(f));
                }
                if self.find_next() {
                    self.scroll_to_caret(ctx);
                }
                Reply::redraw_all()
            }
            form::Event::Pressed(finddlg::REPLACE) => {
                let mut hit = false;
                if let Some(f) = &self.replace {
                    let (query, opts) = replace_query(f);
                    let with = Notepad::field(f, finddlg::WITH);
                    let match_case = opts.match_case;
                    self.last_query = Some((query.clone(), opts));
                    hit = self.replace_once(&query, &with, match_case);
                }
                if hit {
                    self.scroll_to_caret(ctx);
                }
                Reply::redraw_all()
            }
            form::Event::Pressed(finddlg::REPLACE_ALL) => {
                let mut hit = false;
                if let Some(f) = &self.replace {
                    let (query, opts) = replace_query(f);
                    let with = Notepad::field(f, finddlg::WITH);
                    hit = self.replace_every(&query, &with, &opts) > 0;
                    self.last_query = Some((query, opts));
                }
                if hit {
                    self.scroll_to_caret(ctx);
                }
                Reply::redraw_all()
            }
            form::Event::Changed(finddlg::QUERY) => {
                if let Some(f) = &mut self.replace {
                    sync_actions(f);
                }
                Reply::redraw_all()
            }
            form::Event::None => Reply::none(),
            _ => Reply::redraw(),
        }
    }

    pub(crate) fn notfound_event(&mut self, ev: Event) -> Reply {
        let Some(b) = &mut self.notfound else {
            return Reply::none();
        };
        match b.event(&ev, &self.dialog_text, &self.forms) {
            oneshot::Outcome::Closed(_) => {
                self.notfound = None;
                Reply::redraw_all()
            }
            oneshot::Outcome::Redraw => Reply::redraw(),
            oneshot::Outcome::None => Reply::none(),
        }
    }
}
