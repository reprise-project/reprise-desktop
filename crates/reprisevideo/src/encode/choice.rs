//! Which backend codes H.264 for `[remote_desktop] encoder`, and how the status names it.

use super::Codec;
use super::probe::Backend;
use repriseconf::remote::EncoderChoice;

/// The status while bitmaps are sent because no H.264 encoder works.
pub const BITMAP_LABEL: &str = "RemoteFX (no H.264 encoder)";
/// The status while bitmaps are sent because `none` is chosen.
pub const NONE_LABEL: &str = "RemoteFX";
/// The status while the probe at start has not yet found what codes.
pub const PROBING_LABEL: &str = "Finding an H.264 encoder";

/// The backends to try in turn for `choice`: the chosen one if `working`, then the other working
/// ones in auto's order. Empty (bitmaps only) for `none`.
pub fn order(choice: EncoderChoice, working: &[Backend]) -> Vec<Backend> {
    if choice == EncoderChoice::None {
        return Vec::new();
    }
    let chosen = working.iter().filter(|b| b.choice() == choice);
    let others = working.iter().filter(|b| b.choice() != choice);
    chosen.chain(others).copied().collect()
}

/// The `Encoder` status for `choice` while `using` codes (None: bitmaps), naming the chosen
/// encoder another stands in for.
pub fn label(choice: EncoderChoice, using: Option<Backend>) -> String {
    match using {
        None if choice == EncoderChoice::None => NONE_LABEL.into(),
        None => BITMAP_LABEL.into(),
        Some(b) if choice == EncoderChoice::Auto || b.choice() == choice => {
            b.label(Codec::H264).into()
        }
        Some(b) => format!(
            "{} ({} unavailable)",
            b.label(Codec::H264),
            short_name(choice)
        ),
    }
}

/// The `Encoder` status for `choice` with no client connected: the first of its order among
/// `working`.
pub fn configured(choice: EncoderChoice, working: &[Backend]) -> String {
    label(choice, order(choice, working).first().copied())
}

/// The `Encoders` property: the choices `working` offers besides `auto`, `none` always last.
pub fn names(working: &[Backend]) -> Vec<String> {
    (EncoderChoice::ALL.into_iter())
        .filter(|&c| working.iter().any(|b| b.choice() == c) || c == EncoderChoice::None)
        .map(|c| c.name().to_string())
        .collect()
}

fn short_name(choice: EncoderChoice) -> &'static str {
    match choice {
        EncoderChoice::Vaapi => "VA-API",
        EncoderChoice::Nvenc => "NVENC",
        EncoderChoice::Vulkan => "Vulkan",
        EncoderChoice::Software => Codec::H264.software_name(),
        EncoderChoice::Auto | EncoderChoice::None => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VAAPI: Backend = Backend::Vaapi { low_power: false };

    #[test]
    fn a_chosen_encoder_that_is_unavailable_falls_back_to_auto() {
        let working = [VAAPI, Backend::Software];
        assert_eq!(
            order(EncoderChoice::Nvenc, &working),
            [VAAPI, Backend::Software]
        );
        assert_eq!(
            label(EncoderChoice::Nvenc, Some(VAAPI)),
            "VA-API H.264 (NVENC unavailable)"
        );
        assert_eq!(
            order(EncoderChoice::Software, &working),
            [Backend::Software, VAAPI],
            "the chosen one first, then auto's order"
        );
        assert_eq!(
            label(EncoderChoice::Software, Some(Backend::Software)),
            "Software H.264"
        );
        assert_eq!(
            label(EncoderChoice::Software, Some(VAAPI)),
            "VA-API H.264 (x264 unavailable)"
        );
        assert_eq!(
            order(EncoderChoice::Auto, &working),
            [VAAPI, Backend::Software],
            "a VA-API failure goes on to x264, not straight to bitmaps"
        );
        assert_eq!(label(EncoderChoice::Auto, Some(VAAPI)), "VA-API H.264");
        assert_eq!(
            order(EncoderChoice::Vaapi, &working),
            [VAAPI, Backend::Software]
        );
        assert_eq!(order(EncoderChoice::Vulkan, &[]), [], "nothing works");
        assert_eq!(label(EncoderChoice::Vulkan, None), BITMAP_LABEL);
    }

    #[test]
    fn none_means_bitmaps_only() {
        assert_eq!(order(EncoderChoice::None, &[VAAPI, Backend::Software]), []);
        assert_eq!(label(EncoderChoice::None, None), "RemoteFX");
    }

    #[test]
    fn the_bus_lists_what_works_and_none() {
        let low_power = Backend::Vaapi { low_power: true };
        assert_eq!(
            names(&[low_power, Backend::Software]),
            ["vaapi", "software", "none"]
        );
        assert_eq!(names(&[]), ["none"]);
    }
}
