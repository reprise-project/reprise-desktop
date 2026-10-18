//! The two dialogs Options opens, as forms: Properties (which device and
//! view, and which lines show) and Advanced Controls (tone and the other
//! controls a sound card's mixer may carry). Nothing here draws or
//! writes: the host builds them, feeds them events, and reads back the
//! [`Options`] Properties settles on.

use repriseaudio::Snapshot;
use reprisekit::controls::combo::{COMBO_H, Combo};
use reprisekit::controls::form::{Control, Form, Item};
use reprisekit::controls::listbox::ListBox;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::DialogFont;
use reprisekit::trackbar::{self, TrackBar};

use crate::view::{self, Options, View};

pub const PROPS: (i32, i32) = (336, 407);

pub const DEVICE: &str = "device";
const DEVICE_LABEL: &str = "device_label";
pub const PLAYBACK: &str = "playback";
pub const RECORDING: &str = "recording";
pub const OTHER: &str = "other";
pub const OTHER_LIST: &str = "other_list";
pub const SHOWN: &str = "shown";
pub const OK: &str = "ok";
pub const CANCEL: &str = "cancel";
const RADIOS: &str = "adjust";
const DEVICE_ROWS: usize = 8;

pub struct Props {
    pub form: Form,
    keys: Vec<String>,
    devices: Vec<String>,
    hidden: std::collections::BTreeSet<String>,
}

impl Props {
    /// Properties as `o` has things now, for the devices and programs in
    /// `snap`.
    pub fn new(snap: &Snapshot, o: &Options) -> Props {
        let devices: Vec<String> = snap.sinks.iter().map(|d| d.name.clone()).collect();
        let labels: Vec<&str> = snap.sinks.iter().map(|d| d.label.as_str()).collect();
        let picked = view::sink(snap, o.device.as_deref())
            .and_then(|d| devices.iter().position(|n| *n == d.name))
            .unwrap_or(0);
        let mut combo = Combo::new(&labels, picked, labels.len().min(DEVICE_ROWS));
        combo.enabled = !labels.is_empty() && o.view == View::Playback;
        let live = combo.enabled;
        // The Other radio's list: nothing maps to a third kind of line.
        let mut other = Combo::new(&[], 0, 1);
        other.enabled = false;
        let items = vec![
            Item::label_at(DEVICE_LABEL, 11, 14, 70, "&Mixer device:").enabled(live),
            Item::new(
                DEVICE,
                Rect::new(83, 14, 243, COMBO_H),
                Control::Combo(combo),
            ),
            Item::group(
                "adjust_group",
                Rect::new(11, 48, 317, 119),
                "Adjust volume for",
            ),
            Item::radio(PLAYBACK, 21, 71, "&Playback", RADIOS).checked(o.view == View::Playback),
            Item::radio(RECORDING, 21, 100, "&Recording", RADIOS)
                .checked(o.view == View::Recording),
            Item::radio(OTHER, 21, 131, "&Other", RADIOS).enabled(false),
            Item::new(
                OTHER_LIST,
                Rect::new(83, 131, 233, COMBO_H),
                Control::Combo(other),
            ),
            Item::label_at(
                "shown_label",
                11,
                178,
                317,
                "S&how the following volume controls:",
            ),
            Item::new(
                SHOWN,
                Rect::new(11, 199, 317, 156),
                Control::ListBox {
                    list: ListBox::new(Vec::new(), None),
                    icons: Vec::new(),
                },
            ),
            Item::button(OK, Rect::new(171, 368, 75, 23), "OK", true, true),
            Item::button(CANCEL, Rect::new(252, 368, 75, 23), "Cancel", true, false),
        ];
        let mut p = Props {
            // Set in the second dialog font, as the mixer is.
            form: Form::new(items).set_in(DialogFont::ShellDlg),
            keys: Vec::new(),
            devices,
            hidden: o.hidden.clone(),
        };
        p.fill(snap, o.view);
        p.form.focus_id(DEVICE);
        p
    }

    pub fn view(&self) -> View {
        if self.form.checked(RECORDING) == Some(true) {
            View::Recording
        } else {
            View::Playback
        }
    }

    fn list(&mut self) -> Option<&mut ListBox> {
        match &mut self.form.item_mut(SHOWN)?.control {
            Control::ListBox { list, .. } => Some(list),
            _ => None,
        }
    }

    fn fill(&mut self, snap: &Snapshot, view: View) {
        let lines = view::all_lines(
            snap,
            &Options {
                view,
                ..Options::new(view)
            },
        );
        self.keys = lines.iter().map(|l| l.key.clone()).collect();
        let checks = self.keys.iter().map(|k| !self.hidden.contains(k)).collect();
        if let Some(list) = self.list() {
            *list = ListBox::new(lines.into_iter().map(|l| l.title).collect(), None);
            list.checks = checks;
        }
    }

    fn read_list(&mut self) {
        let checks = self.list().map(|l| l.checks.clone()).unwrap_or_default();
        for (k, on) in self.keys.iter().zip(checks) {
            if on {
                self.hidden.remove(k);
            } else {
                self.hidden.insert(k.clone());
            }
        }
    }

    /// A control changed: a radio refills the list with its view's lines
    /// (the ticks so far kept) and greys the device combo and its label
    /// off Playback.
    pub fn changed(&mut self, id: &str, snap: &Snapshot) {
        match id {
            PLAYBACK | RECORDING => {
                self.read_list();
                let v = self.view();
                self.fill(snap, v);
                let live = v == View::Playback && !self.devices.is_empty();
                self.form.set_enabled(DEVICE, live);
                self.form.set_enabled(DEVICE_LABEL, live);
            }
            SHOWN => self.read_list(),
            _ => {}
        }
    }

    /// What OK settles on, from `o`: the view, the output device and the
    /// hidden lines. Advanced Controls is not the dialog's.
    pub fn options(&mut self, o: &Options) -> Options {
        self.read_list();
        let device = match self.form.combo_selected(DEVICE) {
            Some(i) if self.form.item(DEVICE).is_some_and(enabled) => self.devices.get(i).cloned(),
            _ => o.device.clone(),
        };
        Options {
            view: self.view(),
            device,
            hidden: self.hidden.clone(),
            advanced: o.advanced,
        }
    }
}

fn enabled(it: &Item) -> bool {
    matches!(&it.control, Control::Combo(c) if c.enabled)
}

pub const ADVANCED: (i32, i32) = (383, 345);
pub const CLOSE: &str = "close";

/// Advanced Controls for the first column's line: the program's icon and
/// a line on what the dialog is for, Tone Controls with Bass and Treble
/// (each slider between a greyed Low and High), and Other Controls. The
/// sliders are greyed at Low: PipeWire carries no tone controls and no
/// switches of a card's own on its nodes, so nothing maps to them. Set in
/// the second dialog font, its labels laid out at their ink.
pub fn advanced(icon: Option<Canvas>) -> Form {
    let track = |id: &'static str, y: i32| {
        let mut t = TrackBar::new(0, 7);
        t.enabled = false;
        Item::new(
            id,
            Rect::new(154, y, 104, trackbar::HEIGHT),
            Control::TrackBar(t),
        )
    };
    let greyed = |id, x, y, w, text| Item::label_at(id, x, y, w, text).enabled(false);
    Form::new(vec![
        Item::picture("icon", Rect::new(10, 11, 32, 32), icon),
        Item::lines_at(
            "intro",
            67,
            12,
            300,
            &[
                "Use these settings to make small changes to how your",
                "audio sounds.",
            ],
        ),
        Item::group("tone_group", Rect::new(11, 61, 360, 124), "Tone Controls"),
        Item::lines_at(
            "tone_text",
            27,
            79,
            330,
            &["These settings control the tone of your audio."],
        ),
        Item::label_at("bass_label", 27, 102, 40, "&Bass:"),
        greyed("bass_low", 117, 102, 30, "Low"),
        track("bass", 104),
        greyed("bass_high", 274, 102, 30, "High"),
        Item::label_at("treble_label", 27, 147, 40, "&Treble:"),
        greyed("treble_low", 117, 147, 30, "Low"),
        track("treble", 149),
        greyed("treble_high", 274, 147, 30, "High"),
        Item::group(
            "other_group",
            Rect::new(11, 200, 360, 101),
            "Other Controls",
        ),
        Item::lines_at(
            "other_text",
            27,
            220,
            330,
            &[
                "These settings make other changes to how your audio sounds.",
                "This device has none.",
            ],
        ),
        Item::button(CLOSE, Rect::new(296, 312, 75, 23), "Close", true, true),
    ])
    .set_in(DialogFont::ShellDlg)
    .labels_at_ink()
}

#[cfg(test)]
mod tests {
    use super::*;
    use repriseaudio::{App, Channels, Device};

    fn dev(id: u32, name: &str) -> Device {
        Device {
            id,
            name: name.into(),
            label: name.to_uppercase(),
            volume: 0.5,
            mute: false,
            card: None,
            channels: Channels::default(),
            route: None,
        }
    }

    fn snap() -> Snapshot {
        Snapshot {
            sinks: vec![dev(40, "hdmi"), dev(41, "speakers")],
            sources: vec![dev(50, "mic")],
            default_sink: Some("speakers".into()),
            apps: vec![App {
                key: "firefox".into(),
                label: "Firefox".into(),
                icon_hints: Vec::new(),
                binary: None,
                streams: Vec::new(),
                volume: 0.5,
                mute: false,
            }],
            ..Snapshot::default()
        }
    }

    fn rows(p: &Props) -> (Vec<String>, Vec<bool>) {
        match &p.form.item(SHOWN).unwrap().control {
            Control::ListBox { list, .. } => (list.items.clone(), list.checks.clone()),
            _ => unreachable!(),
        }
    }

    /// Properties and Advanced Controls fit their text under every interface and text size.
    #[test]
    fn the_dialogs_fit_their_text() {
        let over = reprisekit::interfaces::form_overflows(|_, _, _| {
            vec![
                (
                    "properties".into(),
                    Props::new(&snap(), &Options::new(View::Playback)).form,
                ),
                ("advanced".into(), advanced(None)),
            ]
        });
        assert!(over.is_empty(), "{}", over.join("\n"));
    }

    #[test]
    fn properties_opens_on_the_view_and_device_in_use() {
        let p = Props::new(&snap(), &Options::new(View::Playback));
        assert_eq!(p.form.checked(PLAYBACK), Some(true));
        assert_eq!(p.form.combo_selected(DEVICE), Some(1), "the default output");
        assert_eq!(
            rows(&p),
            (
                vec!["Volume Control".into(), "Firefox".into()],
                vec![true, true]
            )
        );
        assert!(
            !p.form
                .item(OTHER)
                .is_some_and(|i| matches!(i.control, Control::Radio { enabled: true, .. }))
        );
    }

    #[test]
    fn recording_lists_the_inputs_and_greys_the_device() {
        let mut o = Options::new(View::Playback);
        o.hidden.insert("app:firefox".into());
        let mut p = Props::new(&snap(), &o);
        assert_eq!(rows(&p).1, [true, false], "a hidden line is unticked");
        p.form.focus_id(RECORDING);
        assert_eq!(
            p.form.key(reprisekit::controls::form::Key::Space),
            reprisekit::controls::form::Event::Changed(RECORDING)
        );
        p.changed(RECORDING, &snap());
        assert_eq!(rows(&p), (vec!["MIC".into()], vec![true]));
        assert!(!p.form.item(DEVICE).is_some_and(enabled));
        let out = p.options(&o);
        assert_eq!(out.view, View::Recording);
        assert!(
            out.hidden.contains("app:firefox"),
            "the other view's picks stay"
        );
    }

    #[test]
    fn ok_takes_the_ticks_and_the_device() {
        let o = Options::new(View::Playback);
        let mut p = Props::new(&snap(), &o);
        p.form.set_combo(DEVICE, 0);
        if let Some(l) = p.list() {
            l.checks[1] = false;
        }
        p.changed(SHOWN, &snap());
        let out = p.options(&o);
        assert_eq!(out.device.as_deref(), Some("hdmi"));
        assert!(out.hidden.contains("app:firefox"));
        assert!(!out.hidden.contains("master"));
    }

    /// Writes target/volume-properties.png: Properties on Playback with
    /// its two lines, then Advanced Controls beside it.
    #[test]
    #[ignore]
    fn render_the_dialogs() {
        use reprisekit::controls::form;
        use reprisekit::text::TextRenderer;
        use reprisekit::theme::Theme;
        let theme = Theme {
            access_keys: true,
            ..Theme::default()
        };
        let text = TextRenderer::load(&theme.fonts);
        let px = theme.fonts.size_px;
        let p = Props::new(&snap(), &Options::new(View::Playback));
        let adv = advanced(None);
        let mut c = Canvas::new(
            (PROPS.0 + ADVANCED.0 + 8) as u32,
            PROPS.1.max(ADVANCED.1) as u32,
        );
        c.fill_rect(c.bounds(), theme.colors.button_face);
        form::draw(&mut c, &text, &theme.paint(), &p.form, true, px);
        let mut a = Canvas::new(ADVANCED.0 as u32, ADVANCED.1 as u32);
        a.fill_rect(a.bounds(), theme.colors.button_face);
        form::draw(&mut a, &text, &theme.paint(), &adv, true, px);
        c.blit_over(PROPS.0 + 8, 0, &a);
        reprisekit::raster::write_preview(&c, "volume-properties.png");
    }

    #[test]
    fn the_dialogs_place_their_controls() {
        let p = Props::new(&snap(), &Options::new(View::Playback));
        let at = |f: &Form, id: &str| f.item(id).expect("item exists").rect;
        assert_eq!(PROPS, (336, 407));
        assert_eq!(at(&p.form, DEVICE), Rect::new(83, 14, 243, COMBO_H));
        assert_eq!(at(&p.form, "adjust_group"), Rect::new(11, 48, 317, 119));
        assert_eq!(at(&p.form, PLAYBACK).y, 71);
        assert_eq!(at(&p.form, RECORDING).y, 100);
        assert_eq!(at(&p.form, OTHER).y, 131);
        assert_eq!(at(&p.form, OTHER_LIST), Rect::new(83, 131, 233, COMBO_H));
        assert_eq!(at(&p.form, SHOWN), Rect::new(11, 199, 317, 156));
        assert_eq!(at(&p.form, OK), Rect::new(171, 368, 75, 23));
        assert_eq!(at(&p.form, CANCEL), Rect::new(252, 368, 75, 23));
        assert_eq!(rows(&p).0.len(), 2);
        match &p.form.item(SHOWN).unwrap().control {
            Control::ListBox { list, .. } => assert_eq!(list.selected, None, "no row picked"),
            _ => unreachable!(),
        }

        let a = advanced(None);
        assert_eq!(ADVANCED, (383, 345));
        assert_eq!(at(&a, "icon"), Rect::new(10, 11, 32, 32));
        assert_eq!(at(&a, "tone_group"), Rect::new(11, 61, 360, 124));
        assert_eq!(at(&a, "bass"), Rect::new(154, 104, 104, trackbar::HEIGHT));
        assert_eq!(at(&a, "treble").y, 149);
        assert_eq!(at(&a, "other_group"), Rect::new(11, 200, 360, 101));
        assert_eq!(at(&a, CLOSE), Rect::new(296, 312, 75, 23));
    }

    #[test]
    fn advanced_controls_are_greyed() {
        let f = advanced(None);
        for id in ["bass", "treble"] {
            assert!(matches!(
                f.item(id).unwrap().control,
                Control::TrackBar(TrackBar { enabled: false, .. })
            ));
        }
        assert_eq!(f.default_button(), f.index_of(CLOSE));
    }
}
