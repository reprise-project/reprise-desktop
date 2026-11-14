//! The encoder a recording opens: the backends in turn, with the stream asked for the next one's
//! buffers while its frames are ones that backend cannot read.

use reprisevideo::encode::Codec;
use reprisevideo::encode::probe::Backend;
use std::time::{Duration, Instant};

/// How long frames a backend cannot read may keep coming after the stream was asked for its
/// buffers before it is asked again, and as long again before the backend after it takes over.
pub const HOLD: Duration = Duration::from_secs(1);

/// A request to the stream: offer `modifiers`, the ones `backend` maps, or offer them `again` so
/// the producer negotiates anew.
#[derive(Debug, PartialEq, Eq)]
pub struct Ask {
    pub backend: Backend,
    pub modifiers: Vec<u64>,
    pub again: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Step {
    /// The frame reads into `Backend`: open it.
    Open(Backend),
    /// Drop the frame, asking the stream for other buffers when Some.
    Wait(Option<Ask>),
    /// Every backend was tried.
    Exhausted,
}

pub struct Backends {
    codec: Codec,
    /// The backends not yet tried, in order, each with the stream modifiers it maps.
    left: Vec<(Backend, Vec<u64>)>,
    /// When the stream was asked for the first one's buffers, and whether it was asked again.
    asked: Option<(Instant, bool)>,
}

impl Backends {
    pub fn new(codec: Codec, left: Vec<(Backend, Vec<u64>)>) -> Backends {
        Backends {
            codec,
            left,
            asked: None,
        }
    }

    /// What to do with a frame that came at `now`; `reads` says whether a backend mapping the
    /// modifiers takes it. An opened backend leaves the turn, so the next call tries the one
    /// after it.
    pub fn next(&mut self, now: Instant, reads: impl Fn(Backend, &[u64]) -> bool) -> Step {
        while let Some((backend, modifiers)) = self.left.first() {
            let backend = *backend;
            if reads(backend, modifiers) {
                self.asked = None;
                self.left.remove(0);
                return Step::Open(backend);
            }
            let ask = |again| {
                let modifiers = modifiers.clone();
                Step::Wait(Some(Ask {
                    backend,
                    modifiers,
                    again,
                }))
            };
            let Some((since, again)) = self.asked else {
                self.asked = Some((now, false));
                return ask(false);
            };
            let held = now.saturating_duration_since(since);
            if held >= 2 * HOLD {
                log::warn!(
                    "the screen's stream sent no buffers {} reads",
                    backend.label(self.codec)
                );
                self.asked = None;
                self.left.remove(0);
                continue;
            }
            if held >= HOLD && !again {
                self.asked = Some((since, true));
                return ask(true);
            }
            return Step::Wait(None);
        }
        Step::Exhausted
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reprisevideo::stream::DRM_FORMAT_MOD_LINEAR;

    const VAAPI: Backend = Backend::Vaapi { low_power: false };
    const Y_TILED: u64 = 0x0100_0000_0000_0002;

    fn order() -> Backends {
        Backends::new(
            Codec::H264,
            vec![
                (VAAPI, vec![DRM_FORMAT_MOD_LINEAR, Y_TILED]),
                (Backend::Software, vec![DRM_FORMAT_MOD_LINEAR]),
            ],
        )
    }

    /// Who reads a Y-tiled dma-buf: a backend mapping the modifier, on a GPU.
    fn tiled(backend: Backend, modifiers: &[u64]) -> bool {
        backend.gpu().is_some() && modifiers.contains(&Y_TILED)
    }

    fn linear(_: Backend, modifiers: &[u64]) -> bool {
        modifiers.contains(&DRM_FORMAT_MOD_LINEAR)
    }

    fn ask(backend: Backend, again: bool) -> Step {
        Step::Wait(Some(Ask {
            backend,
            modifiers: vec![DRM_FORMAT_MOD_LINEAR],
            again,
        }))
    }

    #[test]
    fn a_hardware_encoder_that_fails_to_open_hands_over_to_x264_on_linear_buffers() {
        let mut backends = order();
        let t0 = Instant::now();
        assert_eq!(backends.next(t0, tiled), Step::Open(VAAPI));
        assert_eq!(
            backends.next(t0, tiled),
            ask(Backend::Software, false),
            "VA-API failed: the stream is asked for x264's LINEAR buffers"
        );
        let in_flight = t0 + Duration::from_millis(30);
        assert_eq!(
            backends.next(in_flight, tiled),
            Step::Wait(None),
            "a tiled frame still in flight is dropped without asking again"
        );
        assert_eq!(
            backends.next(t0 + Duration::from_millis(60), linear),
            Step::Open(Backend::Software)
        );
        assert_eq!(backends.next(t0, linear), Step::Exhausted);
    }

    #[test]
    fn the_stream_is_asked_again_after_a_hold_and_the_backend_skipped_after_two() {
        let mut backends = order();
        let t0 = Instant::now();
        backends.next(t0, tiled);
        backends.next(t0, tiled);
        let at = |ms| t0 + Duration::from_millis(ms);
        assert_eq!(backends.next(at(999), tiled), Step::Wait(None));
        assert_eq!(backends.next(at(1000), tiled), ask(Backend::Software, true));
        assert_eq!(
            backends.next(at(1500), tiled),
            Step::Wait(None),
            "asked once again"
        );
        assert_eq!(
            backends.next(at(2000), tiled),
            Step::Exhausted,
            "x264 never got buffers it reads"
        );
    }

    #[test]
    fn a_backend_skipped_for_its_buffers_asks_the_stream_for_the_next_ones() {
        const VULKAN_ONLY: u64 = 0x0300_0000_0000_0001;
        let mut backends = Backends::new(
            Codec::H264,
            vec![
                (Backend::Vulkan, vec![VULKAN_ONLY]),
                (Backend::Software, vec![DRM_FORMAT_MOD_LINEAR]),
            ],
        );
        let t0 = Instant::now();
        let tiled_only = |_: Backend, _: &[u64]| false;
        assert!(matches!(backends.next(t0, tiled_only), Step::Wait(Some(_))));
        assert_eq!(
            backends.next(t0 + 2 * HOLD, tiled_only),
            ask(Backend::Software, false),
            "Vulkan is skipped and x264's buffers are asked for at once"
        );
    }
}
