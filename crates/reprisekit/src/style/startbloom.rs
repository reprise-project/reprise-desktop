//! The glow an orb throws from the icon on it ([`crate::style::Style::start_bloom`]): a look paints it
//! from the icon as drawn, in Start's state, and `style::taskbar` screens it over the orb and the bar
//! round it. Each bloom is painted once per icon, scale, state and strength and kept here. It sits in
//! `style` because core cannot reach the looks that paint it.

use crate::mutex::lock;
use std::sync::{Arc, Mutex};

use crate::raster::Canvas;
use crate::style::State;

/// One state's glow, premultiplied at canvas scale 1 (device px), with its top-left `(x, y)` device px from the icon's top-left.
#[derive(Clone)]
pub(crate) struct Bloom {
    pub(crate) canvas: Canvas,
    pub(crate) x: i32,
    pub(crate) y: i32,
}

/// Paints the glow of `icon` (device px, premultiplied) in `state` at scale `d`, at `strength` percent of the look's own glow.
pub(crate) type BloomPainter = fn(icon: &Canvas, state: State, d: i32, strength: u8) -> Bloom;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IconKey {
    Mark(i32),
    Pixels(u32, u32, u64),
}

impl IconKey {
    pub(crate) fn of(icon: &Canvas) -> IconKey {
        let (w, h) = icon.device_size();
        let hash = icon.pixels.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, &p| {
            (h ^ u64::from(p)).wrapping_mul(0x0000_0100_0000_01b3)
        });
        IconKey::Pixels(w, h, hash)
    }
}

type Key = (IconKey, i32, State, u8);

/// How many blooms a style keeps: three states of the icon in use and of the one before it.
const KEPT: usize = 6;

pub(crate) struct StartBloom {
    paint: BloomPainter,
    kept: Mutex<Vec<(Key, Arc<Bloom>)>>,
}

impl std::fmt::Debug for StartBloom {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StartBloom").finish_non_exhaustive()
    }
}

impl StartBloom {
    pub(crate) fn new(paint: BloomPainter) -> StartBloom {
        StartBloom {
            paint,
            kept: Mutex::new(Vec::new()),
        }
    }

    /// The bloom of the icon `key` names in `state` at scale `d` and `strength` percent, painted from `icon()` only when not kept.
    pub(crate) fn get(
        &self,
        key: IconKey,
        d: i32,
        state: State,
        strength: u8,
        icon: impl FnOnce() -> Canvas,
    ) -> Arc<Bloom> {
        let key = (key, d, state, strength);
        let mut kept = lock(&self.kept);
        let entry = match kept.iter().position(|(k, _)| *k == key) {
            Some(i) => kept.remove(i),
            None => {
                if kept.len() >= KEPT {
                    kept.remove(0);
                }
                (key, Arc::new((self.paint)(&icon(), state, d, strength)))
            }
        };
        let b = entry.1.clone();
        kept.push(entry);
        b
    }

    /// The state and strength of each kept bloom, of any icon at any scale.
    pub(crate) fn states(&self) -> Vec<(State, u8)> {
        let kept = lock(&self.kept);
        (kept.iter())
            .map(|&((_, _, state, strength), _)| (state, strength))
            .collect()
    }

    /// Whether the bloom of `key` at scale `d` in `state` at `strength` is kept.
    pub(crate) fn keeps(&self, key: IconKey, d: i32, state: State, strength: u8) -> bool {
        let kept = lock(&self.kept);
        kept.iter().any(|(k, _)| *k == (key, d, state, strength))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static PAINTED: AtomicUsize = AtomicUsize::new(0);

    fn counting(icon: &Canvas, _: State, _: i32, _: u8) -> Bloom {
        PAINTED.fetch_add(1, Ordering::SeqCst);
        Bloom {
            canvas: icon.clone(),
            x: 0,
            y: 0,
        }
    }

    #[test]
    fn a_bloom_is_painted_once_per_icon_scale_state_and_strength() {
        let b = StartBloom::new(counting);
        let icon = Canvas::new(4, 4);
        let states = [State::Normal, State::Hot, State::Pressed];
        for _ in 0..3 {
            for st in states {
                b.get(IconKey::of(&icon), 1, st, 100, || icon.clone());
            }
        }
        assert_eq!(PAINTED.load(Ordering::SeqCst), 3, "one per state");
        let first = b.get(IconKey::Mark(24), 2, State::Hot, 100, || icon.clone());
        let again = b.get(IconKey::Mark(24), 2, State::Hot, 100, || icon.clone());
        assert!(Arc::ptr_eq(&first, &again), "the kept bloom comes back");
        assert_eq!(PAINTED.load(Ordering::SeqCst), 4, "a new key paints once");
        let half = b.get(IconKey::Mark(24), 2, State::Hot, 50, || icon.clone());
        assert!(!Arc::ptr_eq(&first, &half), "50 is not 100's bloom");
        assert_eq!(
            PAINTED.load(Ordering::SeqCst),
            5,
            "the strength paints once"
        );
        let full = b.get(IconKey::Mark(24), 2, State::Hot, 100, || icon.clone());
        assert!(Arc::ptr_eq(&first, &full), "100's bloom is still kept");
        assert_eq!(PAINTED.load(Ordering::SeqCst), 5);
    }

    #[test]
    fn icons_with_other_pixels_have_other_keys() {
        let a = Canvas::new(4, 4);
        let mut b = Canvas::new(4, 4);
        b.put_raw(1, 1, 0xFFFF_0000);
        assert_eq!(IconKey::of(&a), IconKey::of(&a.clone()));
        assert_ne!(IconKey::of(&a), IconKey::of(&b));
    }
}
