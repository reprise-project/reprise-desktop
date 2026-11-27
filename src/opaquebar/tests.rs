use crate::harness::{Harness, TestClient};

fn onyx(extra: &str) -> (Harness, TestClient) {
    let text = format!("[appearance]\ninterface = \"glass\"\n[glass]\nstyle = \"onyx\"\n{extra}");
    let mut config = crate::config::Config::parse(&text).unwrap();
    config.remember_windows = false;
    let mut h = Harness::with_config(config);
    h.output("TEST-1", 1024, 768);
    let c = h.connect();
    (h, c)
}

/// Each bar's output name and whether it is opaque, after the loop turn ends.
fn opaque(h: &mut Harness) -> Vec<(String, bool)> {
    h.dispatch();
    let mut bars: Vec<(String, bool)> = (h.state.bars.iter())
        .map(|b| (b.output.name(), b.opaque))
        .collect();
    bars.sort();
    bars
}

fn one(on: bool) -> Vec<(String, bool)> {
    vec![("TEST-1".into(), on)]
}

#[test]
fn a_maximized_window_makes_its_bar_opaque_until_it_is_restored() {
    let (mut h, mut c) = onyx("");
    let t = h.map_toplevel(&mut c, "notepad", 300, 200);
    assert_eq!(opaque(&mut h), one(false), "restored: glass");
    h.state.set_maximized(t.id, true);
    h.roundtrip(&mut c);
    assert_eq!(opaque(&mut h), one(true), "maximized: opaque");
    assert!(
        h.state.bars[0].drawn.is_some(),
        "the bar is drawn again on the turn it flips"
    );
    h.state.set_maximized(t.id, false);
    h.roundtrip(&mut c);
    assert_eq!(opaque(&mut h), one(false), "restored again: glass");
}

#[test]
fn maximized_and_restored_within_one_turn_leaves_the_bar_glass() {
    let (mut h, mut c) = onyx("");
    let t = h.map_toplevel(&mut c, "notepad", 300, 200);
    assert_eq!(opaque(&mut h), one(false));
    h.state.set_maximized(t.id, true);
    h.state.set_maximized(t.id, false);
    h.state.turn_end();
    assert!(!h.state.bars[0].opaque, "no opaque frame in between");
}

#[test]
fn a_maximized_window_minimized_closed_or_on_another_desktop_leaves_the_bar_glass() {
    let (mut h, mut c) = onyx("[workspaces]\ncount = 2\n");
    let t = h.map_toplevel(&mut c, "notepad", 300, 200);
    h.state.set_maximized(t.id, true);
    h.roundtrip(&mut c);
    assert_eq!(opaque(&mut h), one(true));
    h.state.set_minimized(t.id, true);
    assert_eq!(opaque(&mut h), one(false), "minimized");
    h.state.set_minimized(t.id, false);
    assert_eq!(opaque(&mut h), one(true), "shown again");
    h.state.move_to_desktop(&[t.id], 1);
    assert_eq!(opaque(&mut h), one(false), "on the other desktop");
    h.state.switch_desktop(1);
    assert_eq!(opaque(&mut h), one(true), "its desktop is shown");
    c.destroy(t);
    h.roundtrip(&mut c);
    assert_eq!(opaque(&mut h), one(false), "closed");
}

#[test]
fn only_the_output_showing_the_maximized_window_turns_opaque() {
    let (mut h, mut c) = onyx("");
    h.output("TEST-2", 800, 600);
    h.roundtrip(&mut c);
    let t = h.map_toplevel(&mut c, "notepad", 300, 200);
    h.state.set_maximized(t.id, true);
    h.roundtrip(&mut c);
    let both = |a, b| vec![("TEST-1".into(), a), ("TEST-2".into(), b)];
    assert_eq!(opaque(&mut h), both(true, false), "maximized on TEST-1");
    let second = h.state.scene.outputs()[1].clone();
    h.state.move_to_output(t.id, &second);
    h.roundtrip(&mut c);
    assert_eq!(opaque(&mut h), both(false, true), "moved to TEST-2");
}

#[test]
fn a_fullscreen_window_counts_as_maximized() {
    let (mut h, mut c) = onyx("");
    let t = h.map_toplevel(&mut c, "player", 300, 200);
    h.state.set_fullscreen(t.id, true);
    h.roundtrip(&mut c);
    assert_eq!(opaque(&mut h), one(true));
    h.state.set_fullscreen(t.id, false);
    h.roundtrip(&mut c);
    assert_eq!(opaque(&mut h), one(false));
}

#[test]
fn onyx_set_false_and_glass_keep_the_bar_glass() {
    let (mut h, mut c) = onyx("opaque_when_maximized = false\n");
    let t = h.map_toplevel(&mut c, "notepad", 300, 200);
    h.state.set_maximized(t.id, true);
    h.roundtrip(&mut c);
    assert_eq!(opaque(&mut h), one(false), "Onyx, set false");
    let mut config = crate::config::Config::parse("[appearance]\ninterface = \"glass\"\n").unwrap();
    config.remember_windows = false;
    let mut h = Harness::with_config(config);
    h.output("TEST-1", 1024, 768);
    let mut c = h.connect();
    let t = h.map_toplevel(&mut c, "notepad", 300, 200);
    h.state.set_maximized(t.id, true);
    h.roundtrip(&mut c);
    assert_eq!(opaque(&mut h), one(false), "Glass");
}
