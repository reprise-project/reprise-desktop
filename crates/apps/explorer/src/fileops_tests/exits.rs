//! Every box leaves the same way by its close box as by Escape ([`Explorer::dismiss`]).

use super::*;
use reprisefs::progress::{Asked, Meter};
use std::sync::Arc;

/// What a box's exit leaves behind.
#[derive(Debug, PartialEq)]
struct Left {
    up: Option<String>,
    boxes: usize,
    undo: Option<&'static str>,
    browse_left: usize,
    connection: bool,
    put: bool,
    job_waits: Option<bool>,
    kept: Option<String>,
}

struct Raised {
    _dir: tempfile::TempDir,
    ex: Explorer,
    ctx: repriseapp::Ctx,
    meter: Option<Arc<Meter>>,
    kept: Option<PathBuf>,
    _steps: Option<std::sync::mpsc::Sender<reprisefs::netshare::mount::Step>>,
}

/// The row of the table each box kind is raised by. A new kind does not compile until it has
/// one; the kinds without a window have none, since nothing can close them.
fn row(d: &Dialog) -> Option<usize> {
    Some(match d {
        Dialog::Message { .. } => 0,
        Dialog::Confirm(_) => 1,
        Dialog::Replace(_) => 2,
        Dialog::ChooseProgram(_) => 3,
        Dialog::AttrConfirm(_) => 4,
        Dialog::Advanced { .. } => 5,
        Dialog::ChangeIcon { .. } => 6,
        Dialog::About(..) => 7,
        Dialog::Browse(_) => 8,
        Dialog::Unlock(_) => 9,
        Dialog::BatchRename(_) => 10,
        Dialog::Connect(_) => 11,
        Dialog::Login(_) => 12,
        Dialog::Question { .. } => 13,
        Dialog::Paste(_) | Dialog::Reading(_) => return None,
    })
}

/// The box of row `i` up over a folder holding a.txt, b.txt and sub; `None` past the last row.
fn raise(i: usize) -> Option<Raised> {
    let dir = tempfile::tempdir().expect("a temp dir");
    let (src, dest) = (dir.path().join("src"), dir.path().join("dest"));
    for d in [&src, &dest] {
        std::fs::create_dir_all(d.join("sub")).expect("mkdir");
        std::fs::write(d.join("a.txt"), "old").expect("write");
        std::fs::write(d.join("b.txt"), "b").expect("write");
    }
    std::fs::write(src.join("a.txt"), "new").expect("write");
    let mut ctx = test_ctx();
    let (mut ex, steps) = match i {
        12 | 13 => {
            let (ex, tx) = crate::shares::tests::connecting("smb://nas/private", &mut ctx);
            (ex, Some(tx))
        }
        _ => (
            crate::tests::test_explorer(Place::Folder(dest.clone()), &mut ctx),
            None,
        ),
    };
    let (mut meter, mut kept) = (None, None);
    let subject = reprisekit::controls::filesheet::Subject::Folder;
    match i {
        0 => drop(ex.message_box("Explorer", "x".into(), &mut ctx)),
        1 => {
            let m = Arc::new(Meter::default());
            m.decide(reprisefs::progress::Decision::Pending);
            ex.ask_failed(Asked::new(m.clone()), "Error", "x".into(), &mut ctx);
            meter = Some(m);
        }
        2 => {
            ex.put_all(
                &[src.join("a.txt")],
                &dest,
                &|_| Some(DropOp::Copy),
                false,
                &mut ctx,
            );
            crate::tests::wake_until(&mut ex, &mut ctx, |ex| ex.dialog().is_some());
            kept = Some(dest.join("a.txt"));
        }
        3 => drop(ex.choose_program_for(&dest.join("a.txt"), false, &mut ctx)),
        4 => {
            let form = crate::props_dialog::attr_form(
                &reprisefs::props::AttrMoves::default(),
                Some(true),
                subject,
                &ex.dialog_text,
                ex.dialog_px,
                reprisekit::theme::TextUnits::ONE,
            );
            ex.raise(Dialog::AttrConfirm(crate::props_dialog::AttrConfirm {
                set: None,
                new_name: None,
                attrs: Default::default(),
                hidden: Some(true),
                close: false,
                form,
            }));
        }
        5 => drop(ex.advanced_attributes(subject, None, &mut ctx)),
        6 => {
            let list = reprisekit::controls::iconlist::IconList::new(Vec::new(), Vec::new(), None);
            let form = reprisekit::controls::filesheet::change_icon_form("hicolor", list, true);
            ex.raise(Dialog::ChangeIcon {
                form,
                name: "sub".into(),
                waits: Default::default(),
            });
        }
        7 => drop(ex.about_box(&mut ctx)),
        8 => {
            ex.sv.selection.set_only(Some(0));
            ex.open_browse(false, &mut ctx);
        }
        9 => {
            let u = repriseapp::unlock::Unlock::open("/dev/sdz1".into(), "Stick", &mut ctx);
            ex.raise(Dialog::Unlock(Box::new(u)));
        }
        10 => {
            ex.run(Command::SelectAll, &mut ctx);
            ex.begin_batch_rename();
        }
        11 => drop(ex.open_connect(&mut ctx)),
        12 => {
            let step = reprisefs::netshare::mount::Step::Login(crate::shares::tests::login(false));
            steps
                .as_ref()
                .expect("a connection")
                .send(step)
                .expect("send");
            ex.poll_connection(&mut ctx);
        }
        13 => {
            ex.take_up();
            ex.raise(Dialog::Question {
                title: "Connect to nas".into(),
                text: "Go on?".into(),
                choices: vec!["Yes".into(), "No".into()],
                about: crate::shares::Asking::Connection,
            });
        }
        _ => return None,
    }
    ex.answered = false;
    assert!(ex.dialog().is_some(), "row {i} raises a box");
    Some(Raised {
        _dir: dir,
        ex,
        ctx,
        meter,
        kept,
        _steps: steps,
    })
}

fn exit(mut r: Raised, ev: repriseapp::Event) -> Left {
    r.ex.event(crate::DIALOG, ev, &mut r.ctx);
    Left {
        up: (r.ex.children().into_iter())
            .find(|c| c.id == crate::DIALOG)
            .map(|c| c.title),
        boxes: r.ex.dialogs.len(),
        undo: r.ex.undo,
        browse_left: r.ex.browse_left.len(),
        connection: r.ex.connection.is_some(),
        put: r.ex.put.is_some(),
        job_waits: r.meter.map(|m| m.asking()),
        kept: r
            .kept
            .map(|p| std::fs::read_to_string(p).unwrap_or_default()),
    }
}

#[test]
fn every_box_leaves_the_same_by_its_close_box_as_by_escape() {
    let mut rows = Vec::new();
    for i in 0.. {
        let Some(r) = raise(i) else {
            break;
        };
        assert_eq!(
            r.ex.dialog().and_then(row),
            Some(i),
            "row {i} raises its kind"
        );
        let closed = exit(r, repriseapp::Event::CloseRequest);
        let escaped = exit(raise(i).expect("the row again"), key_event_escape());
        assert_eq!(closed, escaped, "row {i}");
        assert_eq!(closed.up, None, "row {i} leaves no box");
        rows.push(i);
    }
    assert_eq!(rows.len(), 14, "a row for every kind with a window");
}
