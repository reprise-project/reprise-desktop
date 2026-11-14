use super::*;
use crate::capturepage::{self as cap, ProbeState};
use repriseconf::recording::{AudioCodec, Codec, Probed};
use repriseconf::remote::EncoderChoice;

fn greyed(f: &Form, id: &str) -> Vec<bool> {
    match &f.item(id).unwrap_or_else(|| panic!("{id}")).control {
        Control::Combo(c) => c.items.iter().map(|i| i.greyed).collect(),
        _ => panic!("{id} is a combo"),
    }
}

fn probed() -> Probed {
    Probed {
        video: vec![
            (Codec::H264, EncoderChoice::Vaapi),
            (Codec::H264, EncoderChoice::Software),
            (Codec::H265, EncoderChoice::Software),
        ],
        audio: vec![AudioCodec::Aac, AudioCodec::Opus],
    }
}

fn capture(s: &PropSheet) -> &Form {
    &s.pages[Page::ScreenCapture.index()]
}

#[test]
fn the_moved_check_boxes_live_on_screen_capture() {
    assert_moved(cap::SAVE, Page::Advanced, Page::ScreenCapture);
    assert_moved(cap::RECORD_SOUND, Page::Sound, Page::ScreenCapture);
    let s = built();
    assert!(s.pages[Page::Advanced.index()].item(cap::SAVE).is_none());
    assert!(
        s.pages[Page::Sound.index()]
            .item(cap::RECORD_SOUND)
            .is_none()
    );
    assert_eq!(Page::ScreenCapture.slug(), "screen-capture");
    let mut keys = keys_on(&s, Page::ScreenCapture);
    keys.sort();
    assert_eq!(
        keys,
        [
            "recording.audio_bitrate",
            "recording.audio_codec",
            "recording.audio_vbr",
            "recording.bitrate",
            "recording.codec",
            "recording.encoder",
            "recording.flac_compression",
            "recording.frame_rate",
            "recording.keyframe_interval",
            "recording.quality",
            "recording.rate",
            "screenshot.format",
            "screenshot.jpg_quality",
            "screenshot.jxl_effort",
            "screenshot.jxl_lossless",
            "screenshot.jxl_quality",
            "screenshot.png_compression",
            "screenshot.record_sound",
            "screenshot.save",
        ],
        "Restore Defaults takes the rows the page does not show too"
    );
}

#[test]
fn the_format_rate_and_audio_rows_swap_their_controls() {
    let mut sheet = built();
    let mut got = Settings::default();
    let p = Page::ScreenCapture;
    let f = capture(&sheet);
    for id in [cap::PNG_COMPRESSION, cap::QUALITY, cap::AUDIO_BITRATE] {
        assert!(f.item(id).is_some(), "{id} shows by default");
    }
    sheet.tabs.open = p.index();
    sheet.focus_control(cap::FORMAT);
    pick(&mut sheet, &mut got, p, cap::FORMAT, 2);
    assert_eq!(got.screenshot_format, "jxl");
    let f = capture(&sheet);
    assert!(f.item(cap::JXL_EFFORT).is_some());
    assert!(f.item(cap::PNG_COMPRESSION).is_none());
    assert_eq!(f.checked(cap::JXL_LOSSLESS), Some(true));
    assert!(!enabled(f, cap::JXL_QUALITY), "lossless takes no quality");
    assert_eq!(
        f.focus().map(|i| f.items[i].id),
        Some(cap::FORMAT),
        "the format combo keeps the keyboard"
    );
    tick(&mut sheet, &mut got, p, cap::JXL_LOSSLESS, false);
    assert!(!got.screenshot_jxl_lossless);
    assert!(enabled(capture(&sheet), cap::JXL_QUALITY));
    pick(&mut sheet, &mut got, p, cap::FORMAT, 1);
    assert!(capture(&sheet).item(cap::JPG_QUALITY).is_some());
    assert!(capture(&sheet).item(cap::JXL_EFFORT).is_none());

    pick(&mut sheet, &mut got, p, cap::RATE, 2);
    assert_eq!(got.recording_rate, "constant");
    let f = capture(&sheet);
    assert!(f.item(cap::BITRATE).is_some() && f.item(cap::QUALITY).is_none());
    assert_eq!(
        rows(&sheet, p, cap::FRAME_RATE).0[0],
        "Screen rate (60 at most)"
    );
    pick(&mut sheet, &mut got, p, cap::RATE, 0);
    assert_eq!(rows(&sheet, p, cap::FRAME_RATE).0[0], "Screen rate");

    pick(&mut sheet, &mut got, p, cap::AUDIO_CODEC, 2);
    assert_eq!(got.recording_audio_codec, "flac");
    let f = capture(&sheet);
    assert!(f.item(cap::FLAC_COMPRESSION).is_some());
    assert!(f.item(cap::AUDIO_BITRATE).is_none() && f.item(cap::AUDIO_VBR).is_none());
}

#[test]
fn the_sound_rows_grey_with_record_off_and_vbr_needs_opus() {
    let mut sheet = built();
    let mut got = Settings::default();
    let p = Page::ScreenCapture;
    assert!(!got.screenshot_record_sound);
    for id in [cap::AUDIO_CODEC, cap::AUDIO_BITRATE, cap::AUDIO_VBR] {
        assert!(!enabled(capture(&sheet), id), "{id} greys with no sound");
    }
    tick(&mut sheet, &mut got, p, cap::RECORD_SOUND, true);
    assert!(enabled(capture(&sheet), cap::AUDIO_BITRATE));
    assert!(!enabled(capture(&sheet), cap::AUDIO_VBR), "AAC has no VBR");
    pick(&mut sheet, &mut got, p, cap::AUDIO_CODEC, 1);
    assert!(enabled(capture(&sheet), cap::AUDIO_VBR), "Opus has");
}

#[test]
fn the_software_row_names_the_codecs_cpu_encoder() {
    let mut sheet = built();
    let mut got = Settings::default();
    let p = Page::ScreenCapture;
    assert_eq!(rows(&sheet, p, cap::ENCODER).0[4], "Software (x264)");
    pick(&mut sheet, &mut got, p, cap::CODEC, 1);
    assert_eq!(rows(&sheet, p, cap::ENCODER).0[4], "Software (x265)");
    pick(&mut sheet, &mut got, p, cap::CODEC, 2);
    assert_eq!(rows(&sheet, p, cap::ENCODER).0[4], "Software (SVT-AV1)");
    assert_eq!(
        rows(&sheet, Page::RemoteDesktop, "video_encoder").0[4],
        "Software (x264)",
        "Remote Desktop's table stays as it is"
    );
}

#[test]
fn the_probe_greys_what_does_not_work_for_the_codec_shown() {
    let mut sheet = built();
    let s = Settings {
        recording_codec: "h265".into(),
        ..Settings::default()
    };
    write(&mut sheet, &s, &[], "us");
    let page = &mut sheet.pages[Page::ScreenCapture.index()];
    cap::grey(page, &s, Some(&probed()));
    assert_eq!(greyed(page, cap::CODEC), [false, false, true], "no AV1");
    assert_eq!(
        greyed(page, cap::ENCODER),
        [false, true, true, true, false],
        "Automatic and x265 code H.265"
    );
    assert_eq!(greyed(page, cap::AUDIO_CODEC), [false, false, true]);
    let h264 = Settings::default();
    cap::grey(page, &h264, Some(&probed()));
    assert_eq!(
        greyed(page, cap::ENCODER),
        [false, false, true, true, false]
    );
    cap::grey(page, &h264, None);
    for id in [cap::CODEC, cap::ENCODER, cap::AUDIO_CODEC] {
        assert!(greyed(page, id).iter().all(|g| !g), "{id}: nothing known");
    }
}

#[test]
fn the_status_line_follows_the_probe() {
    let mut sheet = built();
    let av1 = Settings {
        recording_codec: "av1".into(),
        ..Settings::default()
    };
    let page = &mut sheet.pages[Page::ScreenCapture.index()];
    let line = |page: &Form| match &page.item(cap::STATUS).unwrap().control {
        Control::Label { text, .. } => text.clone(),
        _ => unreachable!(),
    };
    for (state, s, want) in [
        (ProbeState::Unknown, &av1, ""),
        (ProbeState::Checking, &av1, "Checking encoders..."),
        (ProbeState::Failed, &av1, "Encoders could not be checked."),
        (
            ProbeState::Done(probed()),
            &av1,
            "No AV1 encoder works here.",
        ),
        (ProbeState::Done(probed()), &Settings::default(), ""),
    ] {
        cap::show_probe(page, s, &state);
        assert_eq!(line(page), want, "{state:?}");
    }
    cap::show_probe(page, &av1, &ProbeState::Failed);
    assert!(
        greyed(page, cap::ENCODER).iter().all(|g| !g),
        "a failed probe greys nothing"
    );
}

#[test]
fn the_screen_capture_page_pins_its_pixels_in_every_look() {
    let row = Rect::new(20, 44, 358, 428);
    let default = with(&Settings::default());
    let s = Settings {
        screenshot_format: "jxl".into(),
        screenshot_jxl_lossless: false,
        recording_rate: "average".into(),
        recording_codec: "h265".into(),
        screenshot_record_sound: true,
        recording_audio_codec: "opus".into(),
        ..Settings::default()
    };
    let mut set = with(&s);
    cap::show_probe(
        &mut set.pages[Page::ScreenCapture.index()],
        &s,
        &ProbeState::Done(probed()),
    );
    let pins = |sheet: &PropSheet| {
        let (dx, dy) = sheet.page_offset();
        let row = Rect::new(row.x + dx, row.y + dy, row.width, row.height);
        let f = capture(sheet);
        [row_pins_at(f, row, 1), row_pins_at(f, row, 2)]
    };
    assert_eq!(
        [pins(&default), pins(&set)],
        [
            [
                [0xbcceb65e937e79cb, 0xc4d7fd21c3526ee2, 0x2bfb45746bfff3d4],
                [0xc040bdb823de0bc8, 0xbe5b168c759c0b38, 0xc20f2c91f2fb205b],
            ],
            [
                [0x26a3338f6f2dc6d1, 0x545683389527b572, 0x7c5fba35f06e5d2a],
                [0x7e39fc154fa82437, 0xf57b9f293edc5978, 0x1cd90d595a1f9937],
            ],
        ],
        "defaults, then JPEG XL lossy, Average, H.265 and Opus after a probe; scale 1, 2; Classic, Blue, Glass"
    );
}
