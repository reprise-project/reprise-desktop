//! The Screen Capture page. The row under the format, rate mode and audio codec swaps with the
//! one picked ([`follow`]); [`show_probe`] greys what `reprise-record --probe` did not list.

use reprisekit::controls::form::{Control, Form, Item};
use reprisekit::controls::propsheet::PropSheet;
use reprisekit::raster::Rect;
use reprisekit::trackbar::{self, TrackBar};

use crate::model::{
    AUDIO_CODECS, Choices, ENCODERS, PNG_COMPRESSIONS, RATE_MODES, RECORD_FRAME_RATES,
    SHOT_FORMATS, Settings, VIDEO_CODECS, software_row,
};
use crate::pages::{Area, GX, X, choice, grey_choices, group, spin};
use crate::registry::Page;
use repriseconf::recording::{Codec, Probed};

pub const SAVE: &str = "screenshot_save";
pub const FORMAT: &str = "screenshot_format";
pub const PNG_COMPRESSION: &str = "screenshot_png_compression";
pub const JPG_QUALITY: &str = "screenshot_jpg_quality";
pub const JXL_LOSSLESS: &str = "screenshot_jxl_lossless";
pub const JXL_QUALITY: &str = "screenshot_jxl_quality";
pub const JXL_EFFORT: &str = "screenshot_jxl_effort";
pub const CODEC: &str = "recording_codec";
pub const ENCODER: &str = "recording_encoder";
/// What the probe found for the codec shown: under way, failed, or no encoder for it.
pub const STATUS: &str = "capture_status";
pub const RATE: &str = "recording_rate";
pub const QUALITY: &str = "recording_quality";
pub const BITRATE: &str = "recording_bitrate";
pub const FRAME_RATE: &str = "recording_frame_rate";
pub const KEYFRAME: &str = "recording_keyframe_interval";
pub const RECORD_SOUND: &str = "screenshot_record_sound";
pub const AUDIO_CODEC: &str = "recording_audio_codec";
pub const AUDIO_BITRATE: &str = "recording_audio_bitrate";
pub const AUDIO_VBR: &str = "recording_audio_vbr";
pub const FLAC_COMPRESSION: &str = "recording_flac_compression";

/// Each quality trackbar's label (JPEG XL's Lossless check box stands in), trackbar and the
/// number right of it.
const JPG_LEVEL: [&str; 3] = ["jpg_quality_label", JPG_QUALITY, "jpg_quality_value"];
const JXL_LEVEL: [&str; 3] = [JXL_LOSSLESS, JXL_QUALITY, "jxl_quality_value"];
const VIDEO_LEVEL: [&str; 3] = [
    "recording_quality_label",
    QUALITY,
    "recording_quality_value",
];

/// What greys with Record what plays off; [`AUDIO_VBR`] also needs Opus.
const AUDIO_CONTROLS: [&str; 7] = [
    "audio_codec_label",
    AUDIO_CODEC,
    "audio_bitrate_label",
    AUDIO_BITRATE,
    "audio_bitrate_units",
    "flac_compression_label",
    FLAC_COMPRESSION,
];

/// The width of the label column; each row's control starts at [`CONTROL_X`].
const LABEL_W: i32 = 100;
const CONTROL_X: i32 = GX + LABEL_W + 4;
const VALUE_W: i32 = 30;
/// Where a row's second control starts, right of a spin and its unit.
const RIGHT_X: i32 = X + 224;

/// The screen rate row's text under Constant bitrate, which codes the screen at most this often.
const CONSTANT_SCREEN_RATE: &str = "Screen rate (60 at most)";

/// The 1..100 trackbar `track` on row `y` from [`CONTROL_X`], its value `value` right of it.
fn bar(a: Area, track: &'static str, value: &'static str, name: &'static str, y: i32) -> [Item; 2] {
    let w = a.track(Area::MIN.gright() - CONTROL_X - VALUE_W - 6);
    let r = Rect::new(CONTROL_X, y - 6, w, trackbar::HEIGHT);
    [
        Item::new(
            track,
            r,
            Control::TrackBar(TrackBar::new(0, 100).paged(10, 10)),
        )
        .named(name),
        Item::value_at(value, r.right() + 6, y, VALUE_W, ""),
    ]
}

/// A label and the [`bar`] it names.
fn level(a: Area, ids: [&'static str; 3], name: &'static str, y: i32, text: &str) -> [Item; 3] {
    let [label, track, value] = ids;
    let [t, v] = bar(a, track, value, name, y);
    [Item::label_at(label, GX, y, LABEL_W, text), t, v]
}

/// A label and the combo of `choices` it names on row `y`, `w` wide at [`Area::MIN`].
fn row(
    a: Area,
    label: &'static str,
    id: &'static str,
    y: i32,
    w: i32,
    text: &str,
    choices: &Choices,
) -> [Item; 2] {
    [
        Item::label_at(label, GX, y, LABEL_W, text),
        choice(id, CONTROL_X, y - 4, a.combo(w), choices),
    ]
}

/// A spin's unit, right of the up-down [`spin`] puts on row `y`.
fn units(id: &'static str, y: i32, text: &str) -> Item {
    Item::label_at(
        id,
        CONTROL_X + reprisekit::controls::updown::WIDTH + 4,
        y,
        50,
        text,
    )
}

/// The page for `s`'s format, rate mode and audio codec in `a`, before any value is pushed in.
pub fn page(s: &Settings, a: Area) -> Form {
    let mut items = vec![
        group(a, "screenshots_group", 44, 96, "Screenshots"),
        Item::check(
            SAVE,
            GX,
            66,
            "Save each screenshot as a file in &Pictures/Screenshots",
        ),
    ];
    let format = row(a, "format_label", FORMAT, 90, 110, "&Format:", SHOT_FORMATS);
    let effort_x = format[1].rect.right() + 12;
    items.extend(format);
    match s.screenshot_format.as_str() {
        "jpg" => items.extend(level(a, JPG_LEVEL, "JPG quality", 114, "&Quality:")),
        "jxl" => {
            items.extend(spin(
                "jxl_effort_label",
                JXL_EFFORT,
                effort_x,
                90,
                44,
                "&Effort:",
                (1, 9),
            ));
            items.push(Item::check(JXL_LOSSLESS, GX, 114, "&Lossless"));
            items.extend(bar(a, JXL_QUALITY, JXL_LEVEL[2], "JPEG XL quality", 114));
        }
        _ => items.extend(row(
            a,
            "png_compression_label",
            PNG_COMPRESSION,
            114,
            110,
            "Co&mpression:",
            PNG_COMPRESSIONS,
        )),
    }

    items.push(group(a, "video_group", 150, 184, "Video"));
    items.extend(row(
        a,
        "codec_label",
        CODEC,
        172,
        130,
        "&Codec:",
        VIDEO_CODECS,
    ));
    items.extend(row(
        a,
        "encoder_label",
        ENCODER,
        196,
        160,
        "E&ncoder:",
        ENCODERS,
    ));
    items.push(Item::label_at(
        STATUS,
        CONTROL_X,
        218,
        a.gright() - CONTROL_X,
        "",
    ));
    items.extend(row(
        a,
        "rate_label",
        RATE,
        238,
        160,
        "&Rate control:",
        RATE_MODES,
    ));
    match s.recording_rate.as_str() {
        "average" | "constant" => {
            items.extend(spin(
                "bitrate_label",
                BITRATE,
                GX,
                262,
                LABEL_W,
                "&Bitrate:",
                (500, 100_000),
            ));
            items.push(units("bitrate_units", 262, "kbit/s"));
        }
        _ => items.extend(level(a, VIDEO_LEVEL, "Recording quality", 262, "Q&uality:")),
    }
    let rates: Vec<&str> = RECORD_FRAME_RATES.iter().map(|r| r.1).collect();
    items.extend([
        Item::label_at("frame_rate_label", GX, 288, LABEL_W, "Frame ra&te:"),
        Item::combo(
            FRAME_RATE,
            CONTROL_X,
            284,
            a.combo(160),
            &rates,
            rates.len(),
        ),
    ]);
    items.extend(spin(
        "keyframe_label",
        KEYFRAME,
        GX,
        312,
        LABEL_W,
        "&Keyframe after:",
        (1, 60),
    ));
    items.push(units("keyframe_units", 312, "seconds"));

    items.extend([
        group(a, "audio_group", 344, 92, "Audio"),
        Item::check(
            RECORD_SOUND,
            GX,
            366,
            "Record &what plays in screen recordings",
        ),
    ]);
    items.extend(row(
        a,
        "audio_codec_label",
        AUDIO_CODEC,
        390,
        130,
        "C&odec:",
        AUDIO_CODECS,
    ));
    if s.recording_audio_codec == "flac" {
        items.extend(spin(
            "flac_compression_label",
            FLAC_COMPRESSION,
            GX,
            414,
            LABEL_W,
            "Compre&ssion:",
            (0, 12),
        ));
    } else {
        items.extend(spin(
            "audio_bitrate_label",
            AUDIO_BITRATE,
            GX,
            414,
            LABEL_W,
            "B&itrate:",
            (32, 512),
        ));
        items.push(units("audio_bitrate_units", 414, "kbit/s"));
        items.push(Item::check(AUDIO_VBR, RIGHT_X, 414, "&Variable bitrate"));
    }
    Form::new(items)
}

/// Whether `page` has the rows `s`'s format, rate mode and audio codec show.
fn shows(page: &Form, s: &Settings) -> bool {
    let format = match s.screenshot_format.as_str() {
        "jpg" => JPG_QUALITY,
        "jxl" => JXL_EFFORT,
        _ => PNG_COMPRESSION,
    };
    let rate = match s.recording_rate.as_str() {
        "average" | "constant" => BITRATE,
        _ => QUALITY,
    };
    let audio = match s.recording_audio_codec.as_str() {
        "flac" => FLAC_COMPRESSION,
        _ => AUDIO_BITRATE,
    };
    [format, rate, audio]
        .iter()
        .all(|id| page.item(id).is_some())
}

/// Rebuild the page when its rows are another format's, rate mode's or audio codec's than `s`'s,
/// keeping the focus; then grey and fill what follows the other controls.
pub fn follow(sheet: &mut PropSheet, s: &Settings) {
    let at = Page::ScreenCapture.index();
    if !shows(&sheet.pages[at], s) {
        let old = &sheet.pages[at];
        let focused = old.focus().and_then(|f| old.items.get(f)).map(|i| i.id);
        sheet.set_page(at, page(s, Area::of(sheet)));
        crate::pages::write_form(&mut sheet.pages[at], s);
        if let Some(f) = focused.filter(|f| sheet.pages[at].item(f).is_some()) {
            sheet.pages[at].focus_id(f);
        }
    }
    let form = &mut sheet.pages[at];
    for id in &JXL_LEVEL[1..] {
        form.set_enabled(id, !s.screenshot_jxl_lossless);
    }
    for (ids, v) in [
        (JPG_LEVEL, s.screenshot_jpg_quality),
        (JXL_LEVEL, s.screenshot_jxl_quality),
        (VIDEO_LEVEL, s.recording_quality),
    ] {
        form.set_text(ids[2], &v.to_string());
    }
    for id in AUDIO_CONTROLS {
        form.set_enabled(id, s.screenshot_record_sound);
    }
    form.set_enabled(
        AUDIO_VBR,
        s.screenshot_record_sound && s.recording_audio_codec == "opus",
    );
    let screen_rate = match s.recording_rate.as_str() {
        "constant" => CONSTANT_SCREEN_RATE,
        _ => RECORD_FRAME_RATES[0].1,
    };
    set_row(form, FRAME_RATE, 0, screen_rate);
    let software = ENCODERS.len() - 1;
    set_row(form, ENCODER, software, &software_row(&s.recording_codec));
}

fn set_row(form: &mut Form, id: &str, row: usize, text: &str) {
    if let Some(Control::Combo(c)) = form.item_mut(id).map(|i| &mut i.control)
        && let Some(item) = c.items.get_mut(row)
    {
        item.text = text.into();
    }
}

/// Where `reprise-record --probe` is: not run yet, running, done, or hung, crashed or missing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProbeState {
    Unknown,
    Checking,
    Done(Probed),
    Failed,
}

/// The codec `s` names; one the recorder does not know reads as its default, H.264.
fn codec(s: &Settings) -> Codec {
    (Codec::ALL.into_iter())
        .find(|c| c.name() == s.recording_codec)
        .unwrap_or(Codec::H264)
}

/// Grey the Codec rows no encoder codes, the Encoder rows (Automatic aside) that do not code the
/// codec `s` picks, and the audio Codec rows the MP4 muxer refuses; `None` greys nothing.
pub fn grey(page: &mut Form, s: &Settings, probed: Option<&Probed>) {
    let all = |choices: &Choices| choices.iter().map(|c| c.0).collect::<Vec<_>>();
    let Some(p) = probed else {
        for (id, choices) in [
            (CODEC, VIDEO_CODECS),
            (ENCODER, ENCODERS),
            (AUDIO_CODEC, AUDIO_CODECS),
        ] {
            grey_choices(page, id, choices, &all(choices), &[]);
        }
        return;
    };
    let codecs: Vec<&str> = p.video.iter().map(|(c, _)| c.name()).collect();
    grey_choices(page, CODEC, VIDEO_CODECS, &codecs, &[]);
    let shown = codec(s);
    let encoders: Vec<&str> = (p.video.iter())
        .filter(|(c, _)| *c == shown)
        .map(|(_, e)| e.name())
        .collect();
    grey_choices(page, ENCODER, ENCODERS, &encoders, &["auto"]);
    let audio: Vec<&str> = p.audio.iter().map(|a| a.name()).collect();
    grey_choices(page, AUDIO_CODEC, AUDIO_CODECS, &audio, &[]);
}

/// [`grey`] by what the probe found, and the status line saying where it is.
pub fn show_probe(page: &mut Form, s: &Settings, probe: &ProbeState) {
    let probed = match probe {
        ProbeState::Done(p) => Some(p),
        _ => None,
    };
    grey(page, s, probed);
    page.set_text(STATUS, &status(s, probe));
}

fn status(s: &Settings, probe: &ProbeState) -> String {
    match probe {
        ProbeState::Unknown => String::new(),
        ProbeState::Checking => "Checking encoders...".into(),
        ProbeState::Failed => "Encoders could not be checked.".into(),
        ProbeState::Done(p) if !p.video.iter().any(|(c, _)| *c == codec(s)) => {
            format!("No {} encoder works here.", codec(s).label())
        }
        ProbeState::Done(_) => String::new(),
    }
}
