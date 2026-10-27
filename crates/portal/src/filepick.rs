//! The file chooser's dialog, run as our own child (`reprise-portal --file`, see `filechooser`): an
//! `Input` on stdin, the `Output` on stdout when it closes. Modal on the caller, with a message box
//! child for "not found" and "replace?".

use crate::choices;
use crate::dialog;
use crate::filechooser::{Input, Kind, Output, save_files_paths};
use repriseapp::filedlg::{Host, Resolved, Step, taken};
use repriseapp::oneshot::{MsgBox, Outcome};
use repriseapp::{App, Ctx, Event, FileDrop, MAIN, Reply, WinId};
use reprisefs::mimetree::MimeTree;
use reprisekit::access::Tree;
use reprisekit::controls::filedlg::{
    self, CLIENT, DlgMode, Event as FileEvent, Fields, FileDlg, FolderId,
};
use reprisekit::controls::{form, msgbox};
use reprisekit::look::Paint;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::TextRenderer;
use reprisekit::theme::Theme;
use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

const MSGBOX: WinId = WinId::named("msgbox");
const EXTRA_PAD: i32 = 8;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Decision {
    Done(Vec<PathBuf>),
    Navigate(FolderId),
    Confirm(PathBuf),
    Error(String),
}

fn decide(
    kind: Kind,
    folder_mode: bool,
    host: &mut Host,
    names: &[String],
    files: &[Vec<u8>],
) -> Decision {
    let mut paths = Vec::new();
    for n in names {
        match host.resolve(n) {
            Err(msg) => return Decision::Error(msg),
            Ok(Resolved::Folder(id, p)) => {
                if !folder_mode {
                    return Decision::Navigate(id);
                }
                paths.push(p);
            }
            Ok(Resolved::File(p)) => {
                if folder_mode || (kind == Kind::Open && !p.exists()) {
                    return Decision::Error(format!(
                        "{} was not found. Check the file name and try again.",
                        p.display()
                    ));
                }
                paths.push(p);
            }
        }
    }
    match kind {
        Kind::SaveFiles => match paths.first().and_then(|d| save_files_paths(d, files)) {
            Some(p) => Decision::Done(p),
            None => Decision::Error("These files cannot be saved here.".into()),
        },
        Kind::Save if paths.len() == 1 && taken(&paths[0]) => Decision::Confirm(paths[0].clone()),
        _ if paths.is_empty() => Decision::Error("Type a file name.".into()),
        _ => Decision::Done(paths),
    }
}

struct Pick {
    input: Input,
    kind: Kind,
    dlg: FileDlg,
    host: Host,
    text: TextRenderer,
    /// The theme the message boxes open at.
    forms: Theme,
    msg: Option<(MsgBox, Option<PathBuf>)>,
    out: Arc<Mutex<Output>>,
    mime_tree: Arc<MimeTree>,
}

impl Pick {
    fn keep_owned(&self) -> impl Fn(&reprisefs::listing::Item) -> bool + Clone + Send + 'static {
        let f = self.input.filters.get(self.dlg.filter()).cloned();
        let tree = self.mime_tree.clone();
        move |i: &reprisefs::listing::Item| {
            f.as_ref()
                .is_none_or(|f| f.keeps(&i.name, i.mime.as_deref(), &tree))
        }
    }

    fn list(&mut self, id: FolderId, ctx: &mut Ctx) {
        let keep = self.keep_owned();
        self.host.list(&mut self.dlg, id, &mut ctx.icons, &keep);
    }

    fn finish(&mut self, paths: Vec<PathBuf>) -> Reply {
        let choices = self
            .dlg
            .extra()
            .map(|f| choices::choice_answers(&self.input.choices, f))
            .unwrap_or_default();
        *self.out.lock().unwrap() = Output {
            paths: paths
                .iter()
                .map(|p| p.as_os_str().as_bytes().to_vec())
                .collect(),
            filter: (!self.input.filters.is_empty()).then(|| self.dlg.filter()),
            choices,
        };
        Reply {
            quit: true,
            ..Default::default()
        }
    }

    fn message(
        &mut self,
        text: String,
        icon: &str,
        yes_no: bool,
        replace: Option<PathBuf>,
        ctx: &mut Ctx,
    ) -> Reply {
        let (buttons, default) = if yes_no {
            (vec!["&Yes".into(), "&No".into()], 1)
        } else {
            (vec!["OK".into()], 0)
        };
        let fields = msgbox::Fields {
            text,
            icon: ctx.icons.get(icon, 32),
            buttons,
            default,
            min_text_h: None,
            min_w: 0,
            max_w: repriseapp::dialog_max_w(),
        };
        self.forms = ctx.forms();
        self.msg = Some((MsgBox::new(self.input.title.clone(), fields), replace));
        Reply::redraw_all()
    }

    fn accept(&mut self, names: Vec<String>, ctx: &mut Ctx) -> Reply {
        let decision = decide(
            self.kind,
            self.input.folder_mode,
            &mut self.host,
            &names,
            &self.input.files,
        );
        match decision {
            Decision::Done(p) => self.finish(p),
            Decision::Navigate(id) => {
                self.list(id, ctx);
                self.dlg.clear_file_name();
                Reply::redraw()
            }
            Decision::Confirm(p) => {
                let text = format!(
                    "{} already exists.\nDo you want to replace it?",
                    p.display()
                );
                self.message(text, "dialog-warning", true, Some(p), ctx)
            }
            Decision::Error(text) => self.message(text, "dialog-error", false, None, ctx),
        }
    }

    fn client(&self) -> Rect {
        let (w, h) = self.dlg.client();
        Rect::new(0, 0, w, h)
    }
}

impl App for Pick {
    fn title(&self) -> String {
        self.input.title.clone()
    }

    fn app_id(&self) -> String {
        "reprise.portal".into()
    }

    fn initial_size(&self) -> (i32, i32) {
        self.dlg.client()
    }

    fn children(&self) -> Vec<repriseapp::Child> {
        self.msg
            .iter()
            .map(|(m, _)| m.child(MSGBOX, &self.text, &self.forms))
            .collect()
    }

    fn event(&mut self, win: WinId, ev: Event, ctx: &mut Ctx) -> Reply {
        if win == MSGBOX {
            let Some((m, replace)) = &mut self.msg else {
                return Reply::none();
            };
            return match m.event(&ev, &self.text, &ctx.forms()) {
                Outcome::None => Reply::none(),
                Outcome::Redraw => Reply::redraw(),
                Outcome::Closed(Some(0)) if replace.is_some() => {
                    let p = replace.clone().expect("checked");
                    self.msg = None;
                    self.finish(vec![p])
                }
                Outcome::Closed(_) => {
                    self.msg = None;
                    Reply::redraw_all()
                }
            };
        }
        if win != MAIN {
            return Reply::none();
        }
        let keep = self.keep_owned();
        self.host
            .ensure_listed(&mut self.dlg, &mut ctx.icons, &keep);
        match self.host.event(
            &mut self.dlg,
            &ev,
            &ctx.text,
            &ctx.forms(),
            ctx.size,
            &ctx.theme.metrics,
        ) {
            Step::Closed | Step::Out(FileEvent::Cancel) => Reply {
                quit: true,
                ..Default::default()
            },
            Step::Redraw => Reply::redraw(),
            Step::Resize => Reply::resize(),
            Step::None | Step::Out(FileEvent::CreateFolder(_)) => Reply::none(),
            Step::Out(FileEvent::NeedListing(id)) => {
                self.list(id, ctx);
                Reply::redraw()
            }
            Step::Out(FileEvent::FilterChanged(_)) => {
                if let Some(id) = self.host.folder {
                    self.list(id, ctx);
                }
                Reply::redraw()
            }
            Step::Out(FileEvent::Accept(names)) => self.accept(names, ctx),
            Step::Out(_) => Reply::redraw(),
        }
    }

    fn draw(&mut self, win: WinId, c: &mut Canvas, ctx: &Ctx) {
        if win == MSGBOX {
            if let Some((m, _)) = &self.msg {
                m.draw(c, &ctx.forms(), &self.text);
            }
            return;
        }
        let r = self.client();
        let forms = ctx.forms();
        self.dlg.draw(
            c,
            &ctx.text,
            &Paint {
                s: 1,
                ..forms.paint()
            },
            r,
            forms.fonts.size_px,
        );
    }
}

impl repriseapp::Extended for Pick {
    fn min_size(&self, win: WinId) -> Option<(i32, i32)> {
        (win == MAIN).then(|| self.dlg.min_client())
    }

    fn foreign_parent(&self) -> Option<String> {
        self.input.parent.clone()
    }

    fn modal(&self) -> bool {
        self.input.modal
    }

    fn drop_file_dialog(&mut self, win: WinId) -> Option<FileDrop<'_>> {
        (win == MAIN).then_some(FileDrop {
            dlg: &mut self.dlg,
            host: &mut self.host,
        })
    }

    fn access(&mut self, win: WinId, tree: &mut Tree, ctx: &Ctx) {
        if win == MSGBOX {
            if let Some((m, _)) = &self.msg {
                m.access(tree, &self.text, &ctx.forms());
            }
            return;
        }
        let r = self.client();
        self.dlg
            .access(tree, r, &ctx.text, ctx.forms().fonts.size_px);
    }
}

/// The Files of type rows: the caller's filters, or one All Files row of the dialog's own (none in
/// folder mode). The own row is never reported back.
fn type_rows(input: &Input) -> Vec<String> {
    if input.filters.is_empty() && !input.folder_mode {
        return vec!["All Files".into()];
    }
    input.filters.iter().map(|f| f.label.clone()).collect()
}

/// The caller's `choices` as rows under Files of type, laid out for text at `px`, 100%.
fn add_choices(
    dlg: &mut FileDlg,
    cs: &[choices::Choice],
    text: &TextRenderer,
    px: u32,
    cm: &reprisekit::metrics::ControlMetrics,
) {
    if cs.is_empty() {
        return;
    }
    let (items, bottom) = choices::choice_rows(
        cs,
        (filedlg::FIELD_X, filedlg::EXTRA_Y),
        filedlg::field_w(CLIENT.0),
        filedlg::extra_w(CLIENT.0),
        text,
        px,
        reprisekit::theme::TextUnits::ONE,
    );
    dlg.set_extra(
        form::Form::new(items),
        bottom - filedlg::EXTRA_Y + EXTRA_PAD,
        cm,
    );
}

pub fn run_dialog() {
    let Some(input) = dialog::read_input::<Input>() else {
        return;
    };
    let kind = input.kind.unwrap_or(Kind::Open);
    let theme = reprisekit::theme::Theme::default();
    let text = TextRenderer::load(&theme.fonts);
    let px = theme.fonts.size_px;
    repriseapp::filedlg::remember_views();
    let mut host = Host::default();
    host.icons.prime(&mut dialog::icons());
    let mode = if input.folder_mode {
        DlgMode::Folder
    } else if kind == Kind::Open {
        DlgMode::Open
    } else {
        DlgMode::Save
    };
    let fields = Fields {
        title: input.title.clone(),
        accept_label: input.accept_label.clone(),
        filters: type_rows(&input),
        mode,
        multiple: input.multiple,
    };
    let start = PathBuf::from(OsStr::from_bytes(&input.start));
    let mut dlg = host.open(&start, fields, dialog::start_forms().text);
    dlg.set_file_name(&input.name);
    if let Some(raw) = &input.name_raw {
        host.alias(input.name.clone(), OsStr::from_bytes(raw).to_os_string());
    }
    dlg.set_filter(input.filter);
    add_choices(
        &mut dlg,
        &input.choices,
        &text,
        px,
        &repriseapp::start_theme().look.controls(),
    );
    let out = Arc::new(Mutex::new(Output::default()));
    repriseapp::run(Box::new(Pick {
        input,
        kind,
        dlg,
        host,
        text,
        forms: dialog::start_forms(),
        msg: None,
        out: out.clone(),
        mime_tree: Arc::new(MimeTree::load()),
    }))
    .unwrap_or_else(|e| e.exit());
    let out = out.lock().unwrap().clone();
    match toml::to_string(&out) {
        Ok(s) => print!("{s}"),
        Err(e) => log::warn!("file answer: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn tmp(tag: &str) -> PathBuf {
        let t = std::env::temp_dir().join(format!("reprise-filepick-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(t.join("sub")).unwrap();
        std::fs::write(t.join("a.txt"), "").unwrap();
        t
    }

    fn host(t: &Path) -> Host {
        let mut h = Host::default();
        h.folder = Some(h.folders.id(t));
        h
    }

    #[test]
    fn a_call_without_filters_shows_all_files() {
        assert_eq!(type_rows(&Input::default()), vec!["All Files"]);
        let folder = Input {
            folder_mode: true,
            ..Default::default()
        };
        assert!(type_rows(&folder).is_empty());
    }

    #[test]
    fn open_accepts_existing_files_and_refuses_missing_ones() {
        let t = tmp("open");
        let mut h = host(&t);
        assert_eq!(
            decide(Kind::Open, false, &mut h, &["a.txt".into()], &[]),
            Decision::Done(vec![t.join("a.txt")])
        );
        assert_eq!(
            decide(Kind::Open, false, &mut h, &["b.txt".into()], &[]),
            Decision::Error(format!(
                "{} was not found. Check the file name and try again.",
                t.join("b.txt").display()
            ))
        );
        assert!(matches!(
            decide(Kind::Open, false, &mut h, &["sub".into()], &[]),
            Decision::Navigate(_)
        ));
        std::fs::remove_dir_all(&t).ok();
    }

    #[test]
    fn save_asks_before_replacing() {
        let t = tmp("save");
        let mut h = host(&t);
        assert_eq!(
            decide(Kind::Save, false, &mut h, &["new.txt".into()], &[]),
            Decision::Done(vec![t.join("new.txt")])
        );
        assert_eq!(
            decide(Kind::Save, false, &mut h, &["a.txt".into()], &[]),
            Decision::Confirm(t.join("a.txt"))
        );
        std::fs::remove_dir_all(&t).ok();
    }

    #[test]
    fn folder_mode_picks_the_open_or_named_folder() {
        let t = tmp("folder");
        let mut h = host(&t);
        assert_eq!(
            decide(Kind::Open, true, &mut h, &[".".into()], &[]),
            Decision::Done(vec![t.clone()])
        );
        assert_eq!(
            decide(Kind::Open, true, &mut h, &["sub".into()], &[]),
            Decision::Done(vec![t.join("sub")])
        );
        std::fs::remove_dir_all(&t).ok();
    }

    #[test]
    fn save_files_numbers_inside_the_picked_folder() {
        let t = tmp("files");
        let mut h = host(&t);
        let files = vec![b"a.txt\0".to_vec(), b"z.txt\0".to_vec()];
        assert_eq!(
            decide(Kind::SaveFiles, true, &mut h, &[".".into()], &files),
            Decision::Done(vec![t.join("a (2).txt"), t.join("z.txt")])
        );
        std::fs::remove_dir_all(&t).ok();
    }

    /// A Save dialog with a long folder name, filter labels, accept label and default name, and
    /// choices with long labels and options.
    #[test]
    fn a_save_dialog_with_long_choices_fits_its_text_under_every_interface() {
        use reprisekit::controls::filedlg::{Crumb, Folder};
        let combo = |id: &str, label: &str, options: &[&str]| choices::Choice {
            id: id.into(),
            label: label.into(),
            options: options
                .iter()
                .enumerate()
                .map(|(i, o)| (i.to_string(), o.to_string()))
                .collect(),
            initial: "0".into(),
        };
        let cs = [
            choices::Choice {
                id: "open".into(),
                label: "Open the exported picture in the image viewer when it is done".into(),
                options: Vec::new(),
                initial: "true".into(),
            },
            combo(
                "encoding",
                "Character encoding:",
                &[
                    "Unicode (UTF-8)",
                    "Western European (Windows-1252)",
                    "Central European (ISO-8859-2)",
                ],
            ),
            combo(
                "endings",
                "Line endings for the saved file:",
                &["Unix and macOS (LF)", "Windows (CR LF)"],
            ),
        ];
        let over = reprisekit::interfaces::overflows(|i, theme, text| {
            let base = reprisekit::interfaces::theme_for(i);
            let mut dlg = FileDlg::new(
                Fields {
                    title: "Export Image".into(),
                    accept_label: dialog::mnemonic("_Export Selected Layers"),
                    filters: vec![
                        "Images (*.png, *.jpg, *.jpeg, *.gif, *.bmp, *.webp, *.svg)".into(),
                        "All Files".into(),
                    ],
                    mode: DlgMode::Save,
                    multiple: false,
                },
                repriseapp::filedlg::places(),
            );
            dlg.set_listing(Folder {
                id: 1,
                parent: Some(0),
                crumbs: vec![Crumb {
                    id: 1,
                    label: "Quarterly Reports for the Northern Region (final drafts, 2026)".into(),
                    depth: 0,
                    icon: None,
                }],
                entries: Vec::new(),
            });
            dlg.set_file_name("Quarterly report for the northern region, second draft.png");
            add_choices(
                &mut dlg,
                &cs,
                text,
                base.fonts.size_px,
                &base.look.controls(),
            );
            dlg.set_text(theme.text);
            dlg.drawn_with(&theme.paint());
            dlg.overflows(text, theme.fonts.size_px)
                .into_iter()
                .map(|(part, by)| ("Save".to_string(), part, by))
                .collect()
        });
        assert!(over.is_empty(), "{}", over.join("\n"));
    }
}
