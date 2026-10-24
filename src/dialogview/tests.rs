use super::{
    blur_cues_without_keyboard, client_part, glass_pane, raise_order, reader_focus, refit_origin,
    row_icon_names, to_local,
};
use crate::dialog::Dialog;
use crate::raster::Rect;

#[test]
fn an_icon_file_not_decoded_yet_is_left_for_a_job_and_shows_nothing() {
    use super::cached;
    use crate::raster::Canvas;
    use reprisekit::icontheme::{IconCache, IconTheme};
    let mut cache = IconCache::new(IconTheme::load("test", &[]));
    let mut undecoded = Vec::new();
    let file = "/srv/icons/app.png";
    assert_eq!(cached(&mut cache, file, 32, &mut undecoded), None);
    assert_eq!(undecoded, [(file.to_string(), 32)]);
    cache.put_file(file, 32, Some(Canvas::new(32, 32)));
    undecoded.clear();
    let shown = cached(&mut cache, file, 32, &mut undecoded);
    assert_eq!(shown.map(|c| c.width), Some(32));
    assert_eq!(cached(&mut cache, file, 16, &mut undecoded), None);
    assert_eq!(
        undecoded,
        [(file.to_string(), 16)],
        "another size is its own decode"
    );
    cache.put_file(file, 16, None);
    undecoded.clear();
    assert_eq!(cached(&mut cache, file, 16, &mut undecoded), None);
    assert!(
        undecoded.is_empty(),
        "a file that failed to decode is not tried again"
    );
}

#[test]
fn an_icon_file_stuck_decoding_holds_one_job_and_the_others_decode_once_it_is_stuck() {
    use super::{Decode, next_decode};
    use crate::fsjobs::{Reads, STUCK};
    let t0 = std::time::Instant::now();
    let dead = ("/media/stick/app.png".to_string(), 32);
    let queued = ("/srv/icons/queued.png".to_string(), 32);
    let good = ("/srv/icons/new.png".to_string(), 32);
    let mut decodes = Reads::default();
    let both = vec![queued.clone(), dead.clone()];
    let Decode::Start(held, files) = next_decode(&mut decodes, both.clone(), t0) else {
        panic!("the first decode starts");
    };
    assert_eq!(files, [dead.clone(), queued.clone()]);
    assert_eq!(
        next_decode(&mut decodes, both.clone(), t0 + STUCK / 4),
        Decode::Wait,
        "files a young job has yet to decode are left to it"
    );
    assert_eq!(
        next_decode(&mut decodes, vec![good.clone()], t0 + STUCK / 4),
        Decode::Retry(STUCK * 3 / 4),
        "a new file waits while the held decode is young"
    );
    assert_eq!(
        next_decode(&mut decodes, vec![dead.clone()], t0 + STUCK * 5),
        Decode::Wait,
        "the file a stuck job is blocked on is not decoded again"
    );
    let all = vec![dead.clone(), queued.clone(), good.clone()];
    let Decode::Start(read, files) = next_decode(&mut decodes, all, t0 + STUCK) else {
        panic!("the other files decode once the held job is stuck");
    };
    assert_eq!(
        files,
        [good, queued],
        "the dead file stays with the held job"
    );
    assert!(decodes.land(read));
    assert!(!decodes.land(held));
}

use reprisekit::controls::form::Key;
use reprisekit::controls::openwith::{Program, ROWS};

#[test]
fn only_the_dialog_with_the_keyboard_keeps_its_underlines() {
    use reprisekit::cues::Cues;
    let mut stack = [Cues::ALL; 3];
    assert!(blur_cues_without_keyboard(stack.iter_mut(), Some(2)));
    assert_eq!(stack.map(|c| c.keys), [false, false, true]);
    assert!(stack[0].focus, "the rectangles stay");
    assert!(
        !blur_cues_without_keyboard(stack.iter_mut(), Some(2)),
        "nothing left to hide"
    );
    assert!(blur_cues_without_keyboard(stack.iter_mut(), None));
    assert_eq!(stack.map(|c| c.keys), [false; 3]);
}

#[test]
fn a_readers_focus_brings_out_the_focus_rectangles_but_not_the_underlines() {
    use crate::dialog::DialogLayout;
    use reprisekit::cues::Cues;
    use reprisekit::theme::Theme;
    let theme = Theme::default();
    let text = reprisekit::text::TextRenderer::load(&theme.fonts);
    let px = theme.fonts.size_px;
    let mut run = Dialog::run(&[]);
    let l = DialogLayout::compute(&theme, &text, &run);
    let b = l.buttons[1];
    let at = (b.x + b.width / 2, b.y + b.height / 2);
    let mut cues = Cues::NONE;
    assert!(reader_focus(&mut run, &mut cues, &l, at, &text, px));
    assert_eq!(run.button_focus, Some(1));
    assert_eq!(
        cues,
        Cues {
            focus: true,
            keys: false
        }
    );
    let mut cues = Cues::NONE;
    assert!(
        reader_focus(&mut run, &mut cues, &l, at, &text, px),
        "a focus already there still brings out its rectangle"
    );
    assert!(cues.focus);
    assert!(
        !reader_focus(&mut run, &mut cues, &l, at, &text, px),
        "nothing changes the second time"
    );
}

#[test]
fn a_windowed_dialog_stacks_with_the_windows_and_others_stand_over_them() {
    use crate::compositor::{DeskScene, Layers};
    use reprisekit::controls::copyprogress::{CopyProgress, Kind};
    use reprisescene::TreeId;
    let mut scene = DeskScene::new();
    let layers = Layers::new(&mut scene);
    let cp = CopyProgress::new(
        Kind::Copy,
        "Downloads",
        "KINGSTON",
        crate::theme::Color::rgb(0, 0, 0),
    );
    let copy = Dialog::copy_progress(cp, "/media/KINGSTON".into());
    let run = Dialog::run(&[]);
    let put = |scene: &mut DeskScene, layer: TreeId| {
        let tree = scene.tree_create(layer);
        let b = scene.buffer_create(tree);
        scene.buffer_set_canvas(b, &[0xff00_0000; 16 * 16], 16, 16);
        tree
    };
    let dialog = put(&mut scene, super::dialog_layer(&copy, &layers));
    let window = put(&mut scene, layers.windows);
    let top = |scene: &DeskScene| {
        let hit = scene
            .node_at(scene.root(), (8.0, 8.0).into())
            .expect("a hit");
        scene.parent(hit.node).expect("a tree")
    };
    assert_eq!(
        top(&scene),
        window,
        "the window raised last covers the dialog"
    );
    scene.raise_to_top(dialog);
    assert_eq!(top(&scene), dialog, "the dialog comes up over the window");
    scene.raise_to_top(window);
    assert_eq!(top(&scene), window, "a clicked window goes over it again");
    scene.raise_to_top(dialog);
    scene.set_enabled(dialog, false);
    assert_eq!(top(&scene), window, "minimised, it lets the window show");

    let menu = super::dialog_layer(&run, &layers);
    assert_eq!(menu, layers.menu);
    let run_tree = put(&mut scene, menu);
    scene.raise_to_top(window);
    assert_eq!(
        top(&scene),
        run_tree,
        "a dialog that is not windowed stays over the windows"
    );
}

#[test]
fn a_glass_dialogs_backdrop_covers_its_frame_only() {
    use crate::dialog::DialogLayout;
    use reprisekit::interface::Interface;
    use reprisekit::interfaces::theme_for;
    let glass = theme_for(Interface::Glass);
    let text = reprisekit::text::TextRenderer::load(&glass.fonts);
    let run = DialogLayout::compute(&glass, &text, &Dialog::run(&[]));
    let f = run.frame.unwrap();
    let pane = glass_pane(&glass, true, &run).expect("glass under the frame");
    assert_eq!(pane.shapes.len(), 1);
    assert_eq!(pane.shapes[0].rect, Rect::new(0, 0, f.total.0, f.total.1));
    assert_eq!(
        pane.shapes[0].corner,
        reprisekit::glass::CORNER,
        "the rounded corners"
    );
    assert_eq!(pane.holes, vec![f.client], "the client stays clear");
    assert_eq!(f.total, run.size, "the frame is the whole dialog");
    assert!(glass_pane(&glass, false, &run).is_none(), "opaque glass");
    let classic = crate::theme::Theme::default();
    let cl = DialogLayout::compute(&classic, &text, &Dialog::run(&[]));
    assert!(glass_pane(&classic, true, &cl).is_none(), "Classic");
    let welcome = DialogLayout::compute(&classic, &text, &Dialog::log_off());
    assert!(glass_pane(&classic, true, &welcome).is_none(), "Welcome");
    let welcome = DialogLayout::compute(&glass, &text, &Dialog::log_off());
    assert!(
        glass_pane(&glass, true, &welcome).is_some(),
        "a Welcome panel under glass is framed"
    );
}

#[test]
fn a_dialogs_backdrop_content_is_its_client_area() {
    use crate::dialog::DialogLayout;
    use crate::raster::Canvas;
    use reprisekit::interface::Interface;
    use reprisekit::interfaces::theme_for;
    let glass = theme_for(Interface::Glass);
    let text = reprisekit::text::TextRenderer::load(&glass.fonts);
    let run = DialogLayout::compute(&glass, &text, &Dialog::run(&[]));
    let client = run.frame.as_ref().unwrap().client;
    let (w, h) = (run.size.0 as u32 * 2, run.size.1 as u32 * 2);
    let mut canvas = Canvas::new(w, h);
    for (i, p) in canvas.pixels.iter_mut().enumerate() {
        *p = 0xff00_0000 | i as u32;
    }
    let (part, at) = client_part(&canvas, &run, 2).expect("a framed dialog");
    assert_eq!(at, client);
    assert_eq!(
        (part.width, part.height),
        (client.width as u32 * 2, client.height as u32 * 2)
    );
    let (x, y) = (client.x as u32 * 2, client.y as u32 * 2);
    assert_eq!(
        part.pixels[0],
        0xff00_0000 | (y * w + x),
        "the client's top-left"
    );
    let classic = crate::theme::Theme::default();
    let welcome = DialogLayout::compute(&classic, &text, &Dialog::log_off());
    assert!(client_part(&canvas, &welcome, 2).is_none(), "Welcome");
}

/// A 404x611 sheet at (900, 469) on 1920x1080 whose usable area shrinks to 1280x692 (150% scale, 28 px
/// taskbar) moves up and left just enough to fit; a fitting one stays, a Welcome panel is left alone.
#[test]
fn a_dialog_left_outside_a_shrunken_output_moves_back_inside() {
    let d = Dialog::run(&[]);
    let area = Rect::new(0, 0, 1280, 692);
    assert_eq!(
        refit_origin(&d, (900, 469), (404, 611), area),
        Some((876, 81))
    );
    assert_eq!(
        refit_origin(&d, (200, 10), (404, 611), area),
        Some((200, 10))
    );
    assert_eq!(
        refit_origin(&d, (900, 469), (404, 611), Rect::default()),
        None
    );
    let welcome = Dialog::log_off();
    assert_eq!(refit_origin(&welcome, (900, 469), (404, 611), area), None);
}

#[test]
fn a_point_left_of_the_layout_origin_maps_to_the_pixel_under_it() {
    // A dialog at x -300 on an output left of the primary: layout -299.5 is inside its first column, -300.5 outside.
    assert_eq!(to_local(-299.5, 10.5, (-300, 10)), (0, 0));
    assert_eq!(to_local(-300.5, 9.5, (-300, 10)), (-1, -1));
    assert_eq!(to_local(12.7, 3.2, (10, 0)), (2, 3));
}

#[test]
fn the_chooser_resolves_icons_for_its_visible_rows_only() {
    let others = (0..40)
        .map(|i| Program {
            id: format!("app{i}.desktop"),
            name: format!("Program {i:02}"),
            icon: Some(format!("icon{i}")),
        })
        .collect();
    let mut d = Dialog::open_with(
        std::path::Path::new("/tmp/a.txt"),
        None,
        vec![],
        others,
        None,
        false,
    );
    let names = row_icon_names(&d);
    assert_eq!(names.len(), ROWS, "one name per visible row");
    assert_eq!(
        names[0],
        (0, "folder".to_string(), 16),
        "the group header draws a folder"
    );
    assert_eq!(names[1], (1, "icon0".to_string(), 16));
    for _ in 0..40 {
        d.key(Key::Down);
    }
    let names = row_icon_names(&d);
    assert_eq!(names.len(), ROWS, "one name per visible row");
    assert_eq!(
        names.first(),
        Some(&(30, "icon29".to_string(), 16)),
        "the list scrolled to its last page"
    );
    assert_eq!(names.last(), Some(&(40, "icon39".to_string(), 16)));
}

#[test]
fn raising_a_child_brings_its_parent_up_under_it() {
    // Run (0), its Browse (1), then a sheet opened on its own (2).
    let links = [(0, None), (1, Some(0)), (2, None)];
    assert_eq!(raise_order(&links, 1), vec![2, 0, 1]);
}

#[test]
fn raising_a_parent_keeps_its_child_on_top() {
    let links = [(0, None), (1, Some(0)), (2, None)];
    assert_eq!(raise_order(&links, 0), vec![2, 0, 1]);
}

#[test]
fn raising_the_top_family_keeps_the_order() {
    let links = [(0, None), (1, None), (2, Some(1)), (3, Some(2))];
    assert_eq!(raise_order(&links, 1), vec![0, 1, 2, 3]);
    assert_eq!(raise_order(&links, 3), vec![0, 1, 2, 3]);
}

#[test]
fn a_child_whose_parent_closed_raises_alone() {
    let links = [(0, None), (5, Some(9)), (2, None)];
    assert_eq!(raise_order(&links, 1), vec![0, 2, 1]);
}

#[test]
fn a_write_of_the_execute_bit_or_launcher_keys_reads_the_sheet_again() {
    let props = |attrs: reprisefs::props::AttrMoves, launcher| super::Props {
        path: "/home/u/Desktop/run.sh".into(),
        name: None,
        attrs,
        hidden: None,
        summary: Vec::new(),
        folder_icon: None,
        launcher,
    };
    let attrs = |read_only, executable| reprisefs::props::AttrMoves {
        read_only,
        executable,
        ..Default::default()
    };
    assert!(props(attrs(None, Some(false)), Vec::new()).rereads());
    assert!(props(attrs(None, Some(true)), Vec::new()).rereads());
    let link = vec![(reprisefs::launcher::Key::Link, "a".to_string())];
    assert!(props(attrs(None, None), link).rereads());
    assert!(
        !props(attrs(Some(true), None), Vec::new()).rereads(),
        "Read-only changes no row"
    );
}
