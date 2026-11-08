use super::*;

#[test]
fn escape_abandons_a_marquee_and_a_divider_drag() {
    let dir = tempfile::tempdir().expect("a temp dir");
    for name in ["a.txt", "b.txt"] {
        std::fs::write(dir.path().join(name), "x").expect("write");
    }
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    let escape = || repriseapp::KeyInput {
        sym: reprisekit::textedit::keysym::ESCAPE,
        utf8: None,
        mods: repriseapp::Mods::default(),
        edit: None,
    };

    let base: std::collections::BTreeSet<usize> = [0].into_iter().collect();
    ex.sv.selection.set = base.clone();
    let frame = ex.layout(&ctx).view;
    let sl = ex.view_layout(frame, &ctx);
    ex.marquee_from = Some(reprisekit::controls::shellview::MarqueeDrag::press(
        &ex.sv,
        &sl,
        (10, 10),
        true,
    ));
    ex.sv.marquee = Some(reprisekit::raster::Rect::new(10, 10, 50, 50));
    ex.sv.selection.set = [0, 1].into_iter().collect();
    ex.key(escape(), &mut ctx);
    assert!(ex.marquee_from.is_none() && ex.sv.marquee.is_none());
    assert_eq!(ex.sv.selection.set, base, "the selection is back");

    ex.resizing = Some((0, 100, 178));
    ex.sv.columns[0].width = 218;
    ex.key(escape(), &mut ctx);
    assert!(ex.resizing.is_none());
    assert_eq!(ex.sv.columns[0].width, 178, "the width is back");
}

#[test]
fn a_marquee_counts_its_entries_in_the_status_bar() {
    let dir = tempfile::tempdir().expect("a temp dir");
    for name in ["a.txt", "b.txt", "c.txt"] {
        std::fs::write(dir.path().join(name), "x").expect("write");
    }
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.sv.set_mode(reprisekit::controls::shellview::Mode::List);
    let sl = ex.view_layout(ex.layout(&ctx).view, &ctx);
    let centre = |i: usize| {
        let c = sl.cells[i];
        (c.x + c.width / 2, c.y + c.height / 2)
    };
    ex.pointer(
        repriseapp::PointerInput::Press {
            x: sl.body.right() - 2,
            y: sl.body.bottom() - 2,
            button: BTN_LEFT,
            time: 5000,
            mods: repriseapp::Mods::default(),
        },
        &mut ctx,
    );
    let (x, y) = centre(1);
    ex.pointer(repriseapp::PointerInput::Motion { x, y }, &mut ctx);
    assert_eq!(ex.status.panels[0].text, "2 object(s) selected");
    let (x, y) = centre(0);
    ex.pointer(repriseapp::PointerInput::Motion { x, y }, &mut ctx);
    assert_eq!(ex.status.panels[0].text, "3 object(s) selected");
}

#[test]
fn a_click_in_the_folders_pane_opens_the_folder() {
    use repriseapp::PointerInput;
    let mut ctx = test_ctx();
    let dir = tempfile::tempdir().expect("a temp dir");
    for d in ["a/inner", "b/inner"] {
        std::fs::create_dir_all(dir.path().join(d)).expect("mkdir");
    }
    let here = Place::Folder(dir.path().to_path_buf());
    let mut ex = test_explorer(here.clone(), &mut ctx);
    ex.settings.show_hidden = true;
    ex.build_tree(&mut ctx);
    ex.show(here.clone(), &mut ctx);
    let (a, b) = (
        Place::Folder(dir.path().join("a")),
        Place::Folder(dir.path().join("b")),
    );
    let aim = |ex: &mut Explorer, place: &Place, ctx: &Ctx| {
        let path = ex.tree_path_of(place).expect("the folder's node");
        let i = ex
            .tree
            .visible()
            .iter()
            .position(|r| r.path == path)
            .expect("a visible row");
        ex.tree.top = i.saturating_sub(1);
        let frame = ex.layout(ctx).tree.expect("the Folders pane");
        let px = ctx.theme.fonts.size_px;
        let tl = treeview::layout(&ex.tree, &ctx.text, px, frame, &ctx.theme.controls());
        let y = tl.body.y + (i - ex.tree.top) as i32 * treeview::ROW_H + 8;
        let x = (tl.body.x..tl.body.right())
            .find(|&x| ex.tree.hit(&tl, x, y) == treeview::Hit::Row(i))
            .expect("the row's icon");
        (x + 2, y, i, path)
    };
    let open = |ex: &Explorer, path: &[usize]| ex.tree.node(path).is_some_and(|n| n.expanded);
    let press = |x, y| PointerInput::Press {
        x,
        y,
        button: BTN_LEFT,
        time: 0,
        mods: repriseapp::Mods::default(),
    };

    let (x, y, i, a_path) = aim(&mut ex, &a, &ctx);
    assert!(ex.pointer(PointerInput::Motion { x, y }, &mut ctx).redraw);
    assert_eq!(ex.tree.hot, Some(i), "a's label is hot");
    ex.pointer(press(x, y), &mut ctx);
    assert_eq!(ex.place, a, "the view went to a");
    assert!(open(&ex, &a_path), "a opened");
    let here_path = ex.tree_path_of(&here).expect("here's node");
    assert!(open(&ex, &here_path), "here holds a, so it stays open");

    let (x, y, _, b_path) = aim(&mut ex, &b, &ctx);
    ex.pointer(press(x, y), &mut ctx);
    assert_eq!(ex.place, b);
    assert!(open(&ex, &b_path), "b opened");
    assert!(!open(&ex, &a_path), "a closed");
    ex.pointer(PointerInput::Leave, &mut ctx);
    assert_eq!(ex.tree.hot, None, "nothing is hot once the pointer leaves");
}

#[test]
fn the_views_scrollbar_lights_under_the_pointer_and_a_leave_clears_it() {
    use repriseapp::PointerInput;
    use reprisekit::pointer::Interaction;
    let dir = tempfile::tempdir().expect("a temp dir");
    for i in 0..200 {
        std::fs::write(dir.path().join(format!("f{i:03}.txt")), "1").expect("write");
    }
    let mut ctx = test_ctx();
    ctx.theme = reprisekit::interfaces::theme_for(reprisekit::interface::Interface::Blue);
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    let view = ex.layout(&ctx).view;
    let sl = ex.view_layout(view, &ctx);
    let (x, y, part) = (view.y..view.bottom())
        .flat_map(|y| (view.x..view.right()).map(move |x| (x, y)))
        .find_map(|(x, y)| {
            let bar = shellview::hit(&ex.sv, &sl, x, y).bar()?;
            (bar.1 != reprisekit::classic::widgets::ScrollHit::Thumb).then_some((x, y, bar))
        })
        .expect("200 files overflow the view");
    assert!(ex.pointer(PointerInput::Motion { x, y }, &mut ctx).redraw);
    assert_eq!(ex.sv.bars.of(part), Interaction::Hot);
    let press = PointerInput::Press {
        x,
        y,
        button: crate::BTN_LEFT,
        time: 0,
        mods: repriseapp::Mods::default(),
    };
    ex.pointer(press, &mut ctx);
    assert_eq!(ex.sv.bars.of(part), Interaction::Pressed);
    ex.pointer(PointerInput::Leave, &mut ctx);
    assert_eq!(
        ex.sv.bars.of(part),
        Interaction::Pressed,
        "held through a leave"
    );
    let release = PointerInput::Release {
        x,
        y,
        button: crate::BTN_LEFT,
        mods: repriseapp::Mods::default(),
    };
    ex.pointer(release, &mut ctx);
    ex.pointer(PointerInput::Leave, &mut ctx);
    assert_eq!(ex.sv.bars.lit(), None);
}

#[test]
fn the_views_thumb_drags_and_its_track_pages() {
    use repriseapp::PointerInput;
    use reprisekit::classic::widgets::BarHit;
    let dir = tempfile::tempdir().expect("a temp dir");
    for i in 0..200 {
        std::fs::write(dir.path().join(format!("f{i:03}.txt")), "1").expect("write");
    }
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.sv.set_mode(shellview::Mode::Icons);
    let view = ex.layout(&ctx).view;
    let find = |ex: &Explorer, ctx: &Ctx, want: fn(shellview::Hit) -> bool| {
        let sl = ex.view_layout(view, ctx);
        (view.y..view.bottom())
            .flat_map(|y| (view.x..view.right()).map(move |x| (x, y)))
            .find(|&(x, y)| want(shellview::hit(&ex.sv, &sl, x, y)))
            .expect("200 files overflow the view")
    };
    let press = |x, y| PointerInput::Press {
        x,
        y,
        button: crate::BTN_LEFT,
        time: 0,
        mods: repriseapp::Mods::default(),
    };
    let release = || PointerInput::Release {
        x: 0,
        y: 0,
        button: crate::BTN_LEFT,
        mods: repriseapp::Mods::default(),
    };
    let (x, y) = find(&ex, &ctx, |h| {
        h == shellview::Hit::VBar(BarHit::PageForward)
    });
    ex.pointer(press(x, y), &mut ctx);
    let paged = ex.sv.top;
    assert!(paged > 0, "the track below the thumb pages on");
    ex.pointer(release(), &mut ctx);
    let (x, y) = find(&ex, &ctx, |h| {
        matches!(h, shellview::Hit::VBar(BarHit::Thumb(_)))
    });
    ex.pointer(press(x, y), &mut ctx);
    assert_eq!(ex.sv.top, paged, "the press on the thumb moves nothing");
    ex.pointer(PointerInput::Motion { x: 0, y: 9000 }, &mut ctx);
    let end = ex.sv.top;
    assert!(end > paged, "dragged past the end, off the view");
    ex.pointer(PointerInput::Motion { x, y: view.y }, &mut ctx);
    assert!(ex.sv.top < end, "and back up");
    ex.pointer(release(), &mut ctx);
    let after = ex.sv.top;
    ex.pointer(PointerInput::Motion { x, y: 9000 }, &mut ctx);
    assert_eq!(ex.sv.top, after, "let go: the motion moves nothing");
}

#[test]
fn a_details_column_lights_under_the_pointer() {
    use repriseapp::PointerInput;
    use reprisekit::pointer::Interaction;
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::write(dir.path().join("a.txt"), "1").expect("write");
    let mut ctx = test_ctx();
    ctx.theme = reprisekit::interfaces::theme_for(reprisekit::interface::Interface::Blue);
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.sv.set_mode(shellview::Mode::Details);
    let view = ex.layout(&ctx).view;
    let sl = ex.view_layout(view, &ctx);
    let (x, y) = (view.y..view.bottom())
        .flat_map(|y| (view.x..view.right()).map(move |x| (x, y)))
        .find(|&(x, y)| shellview::hit(&ex.sv, &sl, x, y) == shellview::Hit::Header(1))
        .expect("a second column");
    assert!(ex.pointer(PointerInput::Motion { x, y }, &mut ctx).redraw);
    assert_eq!(ex.sv.header.of(1), Interaction::Hot);
    let press = PointerInput::Press {
        x,
        y,
        button: crate::BTN_LEFT,
        time: 0,
        mods: repriseapp::Mods::default(),
    };
    ex.pointer(press, &mut ctx);
    assert_eq!(ex.sv.header.of(1), Interaction::Pressed);
    let release = PointerInput::Release {
        x,
        y,
        button: crate::BTN_LEFT,
        mods: repriseapp::Mods::default(),
    };
    ex.pointer(release, &mut ctx);
    assert_eq!(
        ex.sv.header.of(1),
        Interaction::Hot,
        "hot again after the release"
    );
    ex.pointer(PointerInput::Leave, &mut ctx);
    assert_eq!(ex.sv.header.lit(), None);
}

#[test]
fn thumb_buttons_step_back_and_forward() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let sub = dir.path().join("sub");
    std::fs::create_dir(&sub).expect("mkdir");
    let mut ctx = test_ctx();
    let top = Place::Folder(dir.path().to_path_buf());
    let mut ex = test_explorer(top.clone(), &mut ctx);
    let _ = ex.run(Command::GoTo(Place::Folder(sub.clone())), &mut ctx);
    let thumb = |button| repriseapp::PointerInput::Press {
        x: 0,
        y: 0,
        button,
        time: 0,
        mods: repriseapp::Mods::default(),
    };
    ex.pointer(thumb(0x113), &mut ctx);
    assert_eq!(ex.place, top, "Back returns to the folder above");
    ex.pointer(thumb(0x114), &mut ctx);
    assert_eq!(ex.place, Place::Folder(sub), "Forward returns to sub");
}
